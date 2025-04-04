use crate::util::id_generator::iputil;
use anyhow::{Context, Result};
use opentelemetry::global;
use opentelemetry::KeyValue;
use opentelemetry_otlp::SpanExporter;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::runtime;
use opentelemetry_sdk::trace::{Tracer, TracerProvider};
use opentelemetry_semantic_conventions::{
    resource::{DEPLOYMENT_ENVIRONMENT_NAME, SERVICE_NAME, SERVICE_VERSION},
    SCHEMA_URL,
};
use serde::Deserialize;
use std::env;
use std::fs::File;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;
use tracing::Subscriber;
use tracing_subscriber::fmt::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::{filter, prelude::*};

// default name (fixed)
const APP_SERVICE_NAME: &str = env!("CARGO_PKG_NAME");

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
    opentelemetry::global::shutdown_tracer_provider();
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

fn zipkin_tracer_from_env(app_service_name: String) -> Result<Tracer> {
    let addr = env::var("ZIPKIN_ADDR").context("zipkin addr")?;
    global::set_text_map_propagator(opentelemetry_zipkin::Propagator::new());
    opentelemetry_zipkin::new_pipeline()
        .with_service_name(app_service_name)
        .with_collector_endpoint(addr)
        .install_batch(opentelemetry_sdk::runtime::Tokio)
        .map_err(|e| {
            println!("failed to install zipkin tracer: {:?}", e);
            e.into()
        })
}

// Create a Resource that captures information about the entity for which telemetry is recorded.
fn resource(app_service_name: String) -> opentelemetry_sdk::Resource {
    opentelemetry_sdk::Resource::from_schema_url(
        [
            KeyValue::new(SERVICE_NAME, app_service_name),
            KeyValue::new(SERVICE_VERSION, env!("CARGO_PKG_VERSION")),
            KeyValue::new(DEPLOYMENT_ENVIRONMENT_NAME, "development"), // TODO from config
        ],
        SCHEMA_URL,
    )
}
async fn set_otlp_tracer_provider_from_env(app_service_name: String) -> Result<()> {
    let addr: Result<String> = env::var("OTLP_ADDR").context("otlp addr");
    match addr {
        Ok(addr) => {
            let exporter = SpanExporter::builder()
                .with_tonic()
                .with_endpoint(&addr)
                .with_timeout(Duration::from_secs(10))
                .build()?;

            let provider = TracerProvider::builder()
                .with_resource(resource(app_service_name.clone()))
                .with_sampler(opentelemetry_sdk::trace::Sampler::ParentBased(Box::new(
                    opentelemetry_sdk::trace::Sampler::TraceIdRatioBased(1.0),
                )))
                .with_id_generator(opentelemetry_sdk::trace::RandomIdGenerator::default())
                .with_batch_exporter(exporter, runtime::Tokio)
                .build();
            global::set_tracer_provider(provider);
            global::set_text_map_propagator(TraceContextPropagator::new());
            // Ok(Some(provider))
            Ok(())
        }
        Err(_e) => {
            // not specified
            Ok(())
        }
    }
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
    set_otlp_tracer_provider_from_env(app_service_name.clone()).await?;

    let remote_tracer = zipkin_tracer_from_env(app_service_name.clone()).ok();
    let subscriber = Box::new(
        tracing_subscriber::registry()
            .with(filter)
            .with(env_filter)
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
            .with(remote_tracer.map(|t| tracing_opentelemetry::layer().with_tracer(t)))
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
    let _ = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_env_filter(EnvFilter::from_default_env())
        .try_init();
}
