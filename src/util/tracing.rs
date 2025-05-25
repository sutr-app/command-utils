use crate::util::id_generator::iputil;
use anyhow::{Context, Result};
use opentelemetry::global;
use opentelemetry::KeyValue;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::tonic_types;
use opentelemetry_otlp::LogExporter;
use opentelemetry_otlp::MetricExporter;
use opentelemetry_otlp::SpanExporter;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_otlp::WithHttpConfig;
use opentelemetry_otlp::WithTonicConfig;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::BatchSpanProcessor;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_semantic_conventions::resource::{DEPLOYMENT_ENVIRONMENT_NAME, SERVICE_VERSION};
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::OnceCell;
use tracing::Subscriber;
use tracing_subscriber::fmt::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::{filter, prelude::*};

// default name (fixed)
const APP_SERVICE_NAME: &str = env!("CARGO_PKG_NAME");
static GLOBAL_TRACER_PROVIDER: OnceCell<SdkTracerProvider> = OnceCell::const_new();
static GLOBAL_LOGGER_PROVIDER: OnceCell<SdkLoggerProvider> = OnceCell::const_new();
static GLOBAL_METER_PROVIDER: OnceCell<SdkMeterProvider> = OnceCell::const_new();

#[derive(Deserialize, Debug)]
pub struct LoggingConfig {
    pub app_name: Option<String>,
    pub level: Option<String>,
    pub file_name: Option<String>,
    pub file_dir: Option<String>,
    pub use_json: bool,
    pub use_stdout: bool,
}

impl LoggingConfig {
    pub fn new() -> Self {
        Self {
            app_name: None,
            level: None,
            file_name: None,
            file_dir: None,
            use_json: false,
            use_stdout: true,
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        tracing::info!("Use default LoggingConfig.");
        Self::new()
    }
}

pub async fn init_from_env_and_filename(
    prefix: impl Into<String>,
    ext: impl Into<String>,
) -> Result<()> {
    let log_filename = create_filename_with_ip_postfix(prefix, ext);
    // no env, use default
    let conf = load_tracing_config_from_env().unwrap_or_default();
    tracing_init(LoggingConfig {
        file_name: Some(log_filename),
        ..conf
    })
    .await
}

pub fn shutdown_tracer_provider() {
    if let Some(provider) = GLOBAL_TRACER_PROVIDER.get() {
        let _ = provider.shutdown().inspect_err(|e| {
            eprintln!("failed to shutdown tracer provider: {:?}", e);
        });
    }
    if let Some(provider) = GLOBAL_METER_PROVIDER.get() {
        let _ = provider.shutdown().inspect_err(|e| {
            eprintln!("failed to shutdown meter provider: {:?}", e);
        });
    }
    if let Some(provider) = GLOBAL_LOGGER_PROVIDER.get() {
        let _ = provider.shutdown().inspect_err(|e| {
            eprintln!("failed to shutdown logger provider: {:?}", e);
        });
    }
}

pub fn create_filename_with_ip_postfix(
    prefix: impl Into<String>,
    ext: impl Into<String>,
) -> String {
    // filename based on ip
    let ip = iputil::resolve_host_ipv4().unwrap_or(*iputil::IP_LOCAL);
    format!("{}_{:x}.{}", prefix.into(), u32::from(ip.ip()), ext.into())
}
pub fn load_tracing_config_from_env() -> Result<LoggingConfig> {
    envy::prefixed("LOG_")
        .from_env::<LoggingConfig>()
        .context("cannot read logging config from env:")
}
pub async fn tracing_init(conf: LoggingConfig) -> Result<()> {
    let layer = setup_layer_from_logging_config(&conf).await?;
    tracing::subscriber::set_global_default(layer).context("setting default subscriber failed")?;
    Ok(())
}
pub async fn tracing_init_from_env() -> Result<()> {
    match load_tracing_config_from_env() {
        Ok(conf) => tracing_init(conf).await,
        Err(e) => {
            tracing::warn!("failed to load logging config from env: {:?}", e);
            Err(e)
        }
    }
}

// fn zipkin_tracer_from_env(app_service_name: String) -> Result<Tracer> {
//     let addr = env::var("ZIPKIN_ADDR").context("zipkin addr")?;
//     global::set_text_map_propagator(opentelemetry_zipkin::Propagator::new());
//     opentelemetry_zipkin::new_pipeline()
//         .with_service_name(app_service_name)
//         .with_collector_endpoint(addr)
//         .install_batch(opentelemetry_sdk::runtime::Tokio)
//         .map_err(|e| {
//             println!("failed to install zipkin tracer: {:?}", e);
//             e.into()
//         })
// }

// Create a Resource that captures information about the entity for which telemetry is recorded.
fn resource(app_service_name: String) -> opentelemetry_sdk::Resource {
    opentelemetry_sdk::Resource::builder()
        .with_service_name(app_service_name)
        .with_attribute(KeyValue::new(SERVICE_VERSION, env!("CARGO_PKG_VERSION")))
        .with_attribute(KeyValue::new(
            DEPLOYMENT_ENVIRONMENT_NAME,
            env::var("DEPLOYMENT_ENVIRONMENT_NAME").unwrap_or_else(|_| "development".to_string()),
        ))
        .build()
}
async fn set_otlp_tracer_provider_from_env(app_service_name: String) -> Result<()> {
    let addr: Result<String> = env::var("OTLP_ADDR").context("otlp addr");
    let http_addr: Result<String> = env::var("OTLP_HTTP_ADDR").context("otlp http addr");
    let token: Option<String> = env::var("OTLP_AUTH_TOKEN").context("otlp addr").ok();
    // Basic Auth: base64(public_key:secret_key)
    let auth_header = token.map(|t| format!("Basic {}", t));
    match (addr, http_addr) {
        (Ok(addr), _) => {
            let mut metadata = tonic_types::metadata::MetadataMap::new();
            if let Some(auth) = auth_header {
                metadata.insert("Authorization", auth.parse().unwrap());
            }

            let exporter = SpanExporter::builder()
                .with_tonic()
                .with_endpoint(&addr)
                .with_timeout(Duration::from_secs(10))
                .with_metadata(metadata)
                .build()?;

            let provider = SdkTracerProvider::builder()
                .with_resource(resource(app_service_name.clone()))
                // .with_sampler(opentelemetry_sdk::trace::Sampler::ParentBased(Box::new(
                //     opentelemetry_sdk::trace::Sampler::TraceIdRatioBased(1.0),
                // )))
                // .with_id_generator(opentelemetry_sdk::trace::RandomIdGenerator::default())
                .with_batch_exporter(exporter)
                // for test
                // .with_span_processor(
                //     BatchSpanProcessor::builder(exporter)
                //         .with_batch_config(
                //             opentelemetry_sdk::trace::BatchConfigBuilder::default()
                //                 .with_max_queue_size(5)
                //                 .with_max_export_batch_size(2)
                //                 .with_scheduled_delay(Duration::from_millis(100))
                //                 .build(),
                //         )
                //         .build(),
                // )
                .build();
            global::set_tracer_provider(provider.clone());
            GLOBAL_TRACER_PROVIDER.set(provider).ok();
            global::set_text_map_propagator(TraceContextPropagator::new());
            // Ok(Some(provider))
            Ok(())
        }
        (_, Ok(http_addr)) => {
            let mut headers = HashMap::new();
            if let Some(auth) = auth_header {
                headers.insert("Authorization".to_string(), auth);
            }

            let exporter = SpanExporter::builder()
                .with_http()
                .with_endpoint(&http_addr)
                .with_timeout(Duration::from_secs(10))
                .with_headers(headers)
                .build()?;

            let provider = SdkTracerProvider::builder()
                .with_resource(resource(app_service_name.clone()))
                // .with_sampler(opentelemetry_sdk::trace::Sampler::ParentBased(Box::new(
                //     opentelemetry_sdk::trace::Sampler::TraceIdRatioBased(1.0),
                // )))
                // .with_id_generator(opentelemetry_sdk::trace::RandomIdGenerator::default())
                .with_batch_exporter(exporter)
                .build();
            global::set_tracer_provider(provider.clone());
            GLOBAL_TRACER_PROVIDER.set(provider).ok();
            global::set_text_map_propagator(TraceContextPropagator::new());
            // Ok(Some(provider))
            Ok(())
        }
        (_, _) => {
            // not specified
            Ok(())
        }
    }
}

async fn create_otlp_logger_provider_layer_from_env(
    app_service_name: String,
) -> Option<OpenTelemetryTracingBridge<SdkLoggerProvider, opentelemetry_sdk::logs::SdkLogger>> {
    let addr: Result<String> = env::var("OTLP_ADDR").context("otlp addr");
    match addr {
        Ok(addr) => {
            // Get protocol configuration from environment or use default "none" (no log exporter)
            let protocol = env::var("OTLP_LOG_PROTOCOL").unwrap_or_else(|_| "none".to_string());
            let builder = LogExporter::builder();

            // Use specific log endpoint if provided, otherwise use the general OTLP address
            let log_endpoint = env::var("OTLP_LOG_ENDPOINT").unwrap_or_else(|_| addr.clone());

            // Try the specified protocol or auto-detect if set to "auto"
            let exporter = match protocol.as_str() {
                "grpc" => builder
                    .with_tonic()
                    .with_endpoint(&log_endpoint)
                    .with_timeout(Duration::from_secs(10))
                    .build(),
                "http" | "http/protobuf" => builder
                    .with_http()
                    .with_endpoint(&log_endpoint)
                    .with_timeout(Duration::from_secs(10))
                    .build(),
                "auto" => {
                    // Try gRPC first, fall back to HTTP if it fails
                    let grpc_result = builder
                        .clone()
                        .with_tonic()
                        .with_endpoint(&log_endpoint)
                        .with_timeout(Duration::from_secs(10))
                        .build();
                    if grpc_result.is_err() {
                        tracing::debug!(
                            "gRPC log exporter failed, trying HTTP: {:?}",
                            grpc_result.err()
                        );
                        builder
                            .with_http()
                            .with_endpoint(&log_endpoint)
                            .with_timeout(Duration::from_secs(10))
                            .build()
                    } else {
                        grpc_result
                    }
                }
                // include "none"
                _ => {
                    tracing::warn!("OTLP log exporter is disabled.");
                    return None;
                }
            };
            match exporter {
                Ok(exp) => {
                    let provider = SdkLoggerProvider::builder()
                        .with_resource(resource(app_service_name.clone()))
                        .with_batch_exporter(exp)
                        .build();
                    let otel_layer = OpenTelemetryTracingBridge::new(&provider.clone());
                    GLOBAL_LOGGER_PROVIDER.set(provider).ok();
                    Some(otel_layer)
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to create OTLP log exporter: {:?}. Log telemetry will be disabled.",
                        e
                    );
                    None
                }
            }
        }
        Err(_) => {
            // OTLP address not specified
            None
        }
    }
}

async fn set_otlp_meter_provider_from_env(app_service_name: String) -> Result<()> {
    let exporter = MetricExporter::builder().with_tonic().build()?;

    let provider = SdkMeterProvider::builder()
        .with_periodic_exporter(exporter)
        .with_resource(resource(app_service_name.clone()))
        .build();
    global::set_meter_provider(provider.clone());
    GLOBAL_METER_PROVIDER.set(provider).ok();
    Ok(())
}

pub async fn setup_layer_from_logging_config(
    conf: &LoggingConfig,
) -> Result<Box<dyn Subscriber + Send + Sync + 'static>> {
    let lv = tracing::Level::from_str(conf.level.as_ref().unwrap_or(&"INFO".to_string()).as_str())
        .unwrap_or(tracing::Level::INFO);
    let filter = filter::Targets::new().with_default(lv);
    let env_filter = tracing_subscriber::EnvFilter::from_default_env();

    // as a deny filter (DEBUG, but remove noisy logs)
    let dir = conf
        .file_dir
        .as_ref()
        .map(|d| PathBuf::from_str(d).context("Invalid log file directory"))
        .unwrap_or(env::current_dir().map_err(|e| e.into()))?;

    let create_file_fn = || {
        if let Some(file_name) = conf.file_name.as_deref() {
            std::fs::create_dir_all(&dir).expect("create log file directory:");
            Some(File::create(dir.join(file_name)).unwrap_or_else(|_| {
                panic!("create log file to {:?}:", dir.join(file_name).as_os_str())
            }))
        } else {
            None
        }
    };
    let app_service_name = conf
        .app_name
        .clone()
        .unwrap_or_else(|| APP_SERVICE_NAME.to_string());

    set_otlp_meter_provider_from_env(app_service_name.clone()).await?;
    set_otlp_tracer_provider_from_env(app_service_name.clone()).await?;
    let otlp_layer = create_otlp_logger_provider_layer_from_env(app_service_name.clone()).await;
    let filter_otel = EnvFilter::new("info")
        .add_directive("hyper=off".parse().unwrap())
        .add_directive("h2=off".parse().unwrap());
    // .add_directive("tonic=off".parse().unwrap())
    // .add_directive("reqwest=off".parse().unwrap());
    let otlp_layer = otlp_layer.with_filter(filter_otel);

    // let remote_tracer = zipkin_tracer_from_env(app_service_name.clone()).ok();
    let subscriber = Box::new(
        tracing_subscriber::registry()
            .with(filter)
            .with(env_filter)
            .with(otlp_layer)
            .with(match create_file_fn() {
                // for json case
                Some(f) if conf.use_json => Some(
                    Layer::new()
                        .with_writer(f.with_max_level(lv))
                        .with_ansi(false)
                        .json(),
                ),
                _ => None,
            })
            .with(match create_file_fn() {
                // for not json case
                Some(f) if !conf.use_json => Some(
                    Layer::new()
                        .with_writer(f.with_max_level(lv))
                        .with_ansi(false),
                ),
                _ => None,
            })
            // .with(remote_tracer.map(|t| tracing_opentelemetry::layer().with_tracer(t)))
            .with(if !conf.use_json && conf.use_stdout {
                Some(tracing_subscriber::fmt::layer().pretty())
            } else {
                None
            })
            .with(if conf.use_json && conf.use_stdout {
                Some(tracing_subscriber::fmt::layer().json())
            } else {
                None
            }),
    );
    //
    // if conf.use_tokio_console {
    // subscriber = Box::new(subscriber.with(console_layer));
    // }
    Ok(subscriber)
}

// for simple stdout logging
pub fn tracing_init_test(level: tracing::Level) {
    let _ = tracing_subscriber::fmt().with_max_level(level).try_init();
}
