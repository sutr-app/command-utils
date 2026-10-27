// reference: https://github.com/open-telemetry/weaver/blob/dc69528344429cc9ccc903fb3229ad051120c1a1/crates/weaver_forge/Cargo.toml
// SPDX-License-Identifier: Apache-2.0

//! Library to hide details of jaq from the rest of weaver.

use super::cache::checked_time_to_idle;
use anyhow::{Result, anyhow};
use jaq_core::{
    Compiler, Ctx, Filter, Vars,
    data::JustLut,
    load::{Arena, File, Loader, parse::Def},
    unwrap_valr,
};
use jaq_json::{Num, Val};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, LazyLock},
    time::Duration,
};
type JqFileType = ();
type JqData = JustLut<Val>;
/// Filter text and the ordered names of the globals it was compiled with.
type FilterCacheKey = (String, Vec<String>);

/// Memory budget of a compiled filter. Every compiled program embeds the
/// compiled standard-library definitions it links (about 86 KiB measured with
/// jaq 3), plus terms that grow with the filter text (30-45 bytes per byte of
/// text measured; 48 is used to stay on the safe side).
const FILTER_BASE_BYTES: u64 = 86 * 1024;
const FILTER_BYTES_PER_CHAR: u64 = 48;

/// Estimated heap size of a compiled filter, used to weigh cache entries.
pub fn estimated_filter_bytes(filter_expr: &str) -> u32 {
    let bytes = FILTER_BASE_BYTES + FILTER_BYTES_PER_CHAR * filter_expr.len() as u64;
    u32::try_from(bytes).unwrap_or(u32::MAX)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JqFilterCacheConfig {
    /// Upper bound of the estimated memory held by cached filters.
    /// 0 disables caching.
    pub max_bytes: u64,
    /// Drop filters not used for this long. `None` keeps them until evicted
    /// by `max_bytes`. Values beyond [`super::cache::MAX_TIME_TO_IDLE`] also
    /// disable idle expiry.
    pub time_to_idle: Option<Duration>,
}

impl Default for JqFilterCacheConfig {
    fn default() -> Self {
        Self {
            max_bytes: 32 * 1024 * 1024,
            time_to_idle: Some(Duration::from_secs(30 * 60)),
        }
    }
}

/// Cache of compiled jq programs.
///
/// Keys are the filter text and the ordered names bound as globals: jaq binds
/// globals by position, so the same text compiled against a different variable
/// set is a different program. Only programs are cached; values are bound on
/// every call, so changes to the variables are always seen.
pub struct JqFilterCache {
    filters: moka::sync::Cache<FilterCacheKey, Arc<Filter<JqData>>>,
}

impl JqFilterCache {
    pub fn new(config: JqFilterCacheConfig) -> Self {
        let mut builder = moka::sync::Cache::builder()
            .max_capacity(config.max_bytes)
            .weigher(|key: &FilterCacheKey, _| estimated_filter_bytes(&key.0));
        if let Some(tti) = checked_time_to_idle(config.time_to_idle) {
            builder = builder.time_to_idle(tti);
        }
        Self {
            filters: builder.build(),
        }
    }

    /// Run a jq filter. Only the `params` the filter references are exposed,
    /// as `$key`; unreferenced ones are never converted.
    pub fn execute(
        &self,
        input: &serde_json::Value,
        filter_expr: &str,
        params: &BTreeMap<String, Arc<serde_json::Value>>,
    ) -> Result<serde_json::Value> {
        let bound = bound_variables(filter_expr, params);
        let names = bound.iter().map(|(k, _)| (*k).to_owned()).collect();
        // Concurrent misses on the same key compile once; errors are not cached.
        let filter = self
            .filters
            .try_get_with((filter_expr.to_owned(), names), || {
                let names: Vec<String> = bound.iter().map(|(k, _)| (*k).to_owned()).collect();
                compile_filter(filter_expr, &names, params).map(Arc::new)
            })
            .map_err(|e| anyhow!("{e}"))?;
        run_filter(&filter, filter_expr, &bound, input)
    }

    #[cfg(test)]
    fn entry_count(&self) -> u64 {
        self.filters.entry_count()
    }

    #[cfg(test)]
    fn run_pending_tasks(&self) {
        self.filters.run_pending_tasks();
    }
}

/// Cache used by [`execute_jq`]; callers that need their own limits create a
/// [`JqFilterCache`].
static DEFAULT_FILTER_CACHE: LazyLock<JqFilterCache> =
    LazyLock::new(|| JqFilterCache::new(JqFilterCacheConfig::default()));

/// Parsing the standard library dominates compile time, so do it once.
static STD_DEFS: LazyLock<Vec<Def<&'static str>>> = LazyLock::new(|| {
    jaq_core::defs()
        .chain(jaq_std::defs())
        .chain(jaq_json::defs())
        .collect()
});

/// Names of the `$variables` that appear in a jq filter, without the `$`.
///
/// This is a textual scan, so names inside string literals are included too.
/// Over-approximation only binds an extra variable; a real reference always
/// appears in the text, so no referenced variable is ever missed.
pub fn referenced_variables(filter: &str) -> BTreeSet<&str> {
    let bytes = filter.as_bytes();
    let mut names = BTreeSet::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len()
                && (bytes[end] == b'_'
                    || bytes[end].is_ascii_alphabetic()
                    || (end > start && bytes[end].is_ascii_digit()))
            {
                end += 1;
            }
            if end > start {
                names.insert(&filter[start..end]);
            }
            i = end.max(start);
        } else {
            i += 1;
        }
    }
    names
}

/// Variables from `params` that `filter` references, in `params` order.
fn bound_variables<'p>(
    filter: &str,
    params: &'p BTreeMap<String, Arc<serde_json::Value>>,
) -> Vec<(&'p str, &'p serde_json::Value)> {
    let referenced = referenced_variables(filter);
    params
        .iter()
        .filter(|(k, _)| referenced.contains(k.as_str()))
        .map(|(k, v)| (k.as_str(), v.as_ref()))
        .collect()
}

#[cfg(test)]
fn bound_variable_names<'p>(
    filter: &str,
    params: &'p BTreeMap<String, Arc<serde_json::Value>>,
) -> Vec<&'p str> {
    bound_variables(filter, params)
        .into_iter()
        .map(|(k, _)| k)
        .collect()
}

fn json_to_val(value: &serde_json::Value) -> Result<Val> {
    Val::deserialize(value).map_err(|e| anyhow!(e))
}

fn compile_filter(
    filter_expr: &str,
    names: &[String],
    params: &BTreeMap<String, Arc<serde_json::Value>>,
) -> Result<Filter<JqData>> {
    let loader = Loader::new(STD_DEFS.iter().cloned());
    let arena = Arena::default();
    let program: File<&str, JqFileType> = File {
        code: filter_expr,
        path: (),
    };
    let modules = loader
        .load(&arena, program)
        .map_err(load_errors)
        .map_err(|e| anyhow!(e))?;

    let globals: Vec<String> = names.iter().map(|n| format!("${n}")).collect();
    let funs = jaq_core::funs::<JqData>()
        .chain(jaq_std::funs::<JqData>())
        .chain(jaq_json::funs::<JqData>());
    // The `(name, args, native)` map shortens `&'static str` to a borrow that
    // matches `with_global_vars`'s shorter lifetime; without it the compiler
    // unifies on `'static` and rejects `globals` (a local Vec).
    #[allow(clippy::map_identity)]
    let filter = Compiler::<&str, JqData>::default()
        .with_funs(funs.map(|(name, args, native)| (name, args, native)))
        .with_global_vars(globals.iter().map(|s| s.as_str()))
        .compile(modules)
        .map_err(|errs| {
            let available_vars: Vec<&str> = params.keys().map(|s| s.as_str()).collect();
            tracing::debug!(
                "jq compile error for '{}'. Available variables: {:?}",
                filter_expr,
                available_vars
            );
            compile_errors(errs)
        })
        .map_err(|e| anyhow!(e))?;
    Ok(filter)
}

/// Run a jq filter using a process-wide cache with default limits.
pub fn execute_jq(
    input: serde_json::Value,
    filter_expr: &str,
    params: &BTreeMap<String, Arc<serde_json::Value>>,
) -> Result<serde_json::Value> {
    DEFAULT_FILTER_CACHE.execute(&input, filter_expr, params)
}

fn run_filter(
    filter: &Filter<JqData>,
    filter_expr: &str,
    bound: &[(&str, &serde_json::Value)],
    input: &serde_json::Value,
) -> Result<serde_json::Value> {
    let values = bound
        .iter()
        .map(|(k, v)| {
            json_to_val(v).map_err(|e| anyhow!("failed to convert jq context variable ${k}: {e}"))
        })
        .collect::<Result<Vec<_>>>()?;
    let ctx = Ctx::<JqData>::new(&filter.lut, Vars::new(values));

    let input_val =
        json_to_val(input).map_err(|e| anyhow!("failed to convert jq input value: {e}"))?;
    let mut values = Vec::new();
    // `unwrap_valr` strips internal control-flow exceptions (tail call,
    // break) that must never reach the caller, leaving only proper errors.
    //
    // Stop on the first runtime error. jaq-json `fromjson` uses `parse_many`,
    // which peeks an invalid token without consuming it, so the iterator
    // never ends. Collecting those errors (each of which embeds the whole
    // input) is what OOMs workflow export on markdown-fenced LLM output.
    for r in filter.id.run((ctx, input_val)) {
        match unwrap_valr(r) {
            Ok(v) => values.push(val_to_json(v)?),
            Err(e) => {
                return Err(anyhow!("jq runtime error for '{filter_expr}': {e}"));
            }
        }
    }

    if values.len() == 1 {
        return Ok(values.pop().expect("values.len() == 1, should not happen"));
    }

    Ok(serde_json::Value::Array(values))
}

// JAQ errors must be parsed and synthesized.  All of this code is adapted from `jaq/src/main.rs`.

/// Converts all errors from jaq into a single string with comma separator.
fn errors_to_string<Reports: Iterator<Item = String>>(reports: Reports) -> String {
    reports.into_iter().collect::<Vec<_>>().join(", ")
}

/// Turns loading errors from jaq into raw strings.
fn load_errors(errs: jaq_core::load::Errors<&str, JqFileType>) -> String {
    use jaq_core::load::Error;
    let errs = errs.into_iter().flat_map(|(_, err)| {
        let result: Vec<String> = match err {
            Error::Io(errs) => errs.into_iter().map(report_io).collect(),
            Error::Lex(errs) => errs.into_iter().map(report_lex).collect(),
            Error::Parse(errs) => errs.into_iter().map(report_parse).collect(),
        };
        result
    });
    errors_to_string(errs)
}

/// Turns compile errors from jaq into raw strings.
fn compile_errors(errs: jaq_core::compile::Errors<&str, JqFileType>) -> String {
    let errs = errs
        .into_iter()
        .flat_map(|(_, errs)| errs.into_iter().map(report_compile));
    errors_to_string(errs)
}

/// Turns IO errors from JQ into raw strings.
fn report_io((path, error): (&str, String)) -> String {
    format!("could not load file {path}: {error}")
}

/// Turns lexing errors from JQ into raw strings.
fn report_lex((expected, _): jaq_core::load::lex::Error<&str>) -> String {
    format!("expected {}", expected.as_str())
}

/// Turns parsing errors from JQ into raw strings.
fn report_parse((expected, _): jaq_core::load::parse::Error<&str>) -> String {
    format!("expected {}", expected.as_str())
}

/// Turns errors coming from JAQ compile phase into raw strings.
fn report_compile((found, undefined): jaq_core::compile::Error<&str>) -> String {
    use jaq_core::compile::Undefined::Filter;
    let wnoa = |exp, got| format!("wrong number of arguments (expected {exp}, found {got})");
    match (found, undefined) {
        ("reduce", Filter(arity)) => wnoa("2", arity),
        ("foreach", Filter(arity)) => wnoa("2 or 3", arity),
        (sym, Filter(arity)) => format!("undefined filter `{sym}/{arity}`"),
        (sym, undefined) => format!("undefined {} `{sym}`", undefined.as_str()),
    }
}

// jaq-json 2.x dropped its `From<serde_json::Value>` / `Into<serde_json::Value>`
// pair, so the outbound direction is owned here. Numbers go through `Display`
// because `serde_json::Number` can't represent jaq's `BigInt` / `Dec` variants
// without the `arbitrary_precision` feature.
fn val_to_json(v: Val) -> Result<serde_json::Value> {
    use serde_json::Value;
    Ok(match v {
        Val::Null => Value::Null,
        Val::Bool(b) => Value::Bool(b),
        Val::Num(n) => num_to_json(n),
        Val::TStr(b) | Val::BStr(b) => Value::String(String::from_utf8_lossy(&b).into_owned()),
        Val::Arr(rc) => Value::Array(
            rc.iter()
                .map(|v| val_to_json(v.clone()))
                .collect::<Result<_>>()?,
        ),
        Val::Obj(rc) => {
            let mut map = serde_json::Map::with_capacity(rc.len());
            for (k, v) in rc.iter() {
                let key = match k {
                    Val::TStr(b) | Val::BStr(b) => String::from_utf8_lossy(b).into_owned(),
                    _ => return Err(anyhow!("jq object key is not a string: {k:?}")),
                };
                map.insert(key, val_to_json(v.clone())?);
            }
            Value::Object(map)
        }
    })
}

fn num_to_json(n: Num) -> serde_json::Value {
    use serde_json::{Number, Value};
    let from_f64 = |f: f64| Number::from_f64(f).map_or(Value::Null, Value::Number);
    match n {
        Num::Int(i) => Value::Number(Number::from(i as i64)),
        Num::Float(f) => from_f64(f),
        Num::BigInt(_) | Num::Dec(_) => {
            let s = format!("{n}");
            if let Ok(i) = s.parse::<i64>() {
                Value::Number(Number::from(i))
            } else if let Ok(u) = s.parse::<u64>() {
                Value::Number(Number::from(u))
            } else {
                from_f64(s.parse().unwrap_or(f64::NAN))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use super::execute_jq;
    use serde_json::json;

    #[test]
    fn run_jq() {
        let input = json!({
            "key1": 1,
            "key2": 2,
        });
        let values = BTreeMap::new();
        let result = execute_jq(input.clone(), ".", &values).unwrap();
        assert_eq!(input, result);
    }

    #[test]
    fn run_jq_string() {
        let value = "plain string";
        let input = json!({"input": value});
        let values = BTreeMap::new();
        let result = execute_jq(input.clone(), ".input", &values).unwrap();
        assert_eq!(value, result);
    }

    #[test]
    fn run_jq_with_context() {
        let input = json!({
            "key1": 1,
            "key2": {
                "key8": 8,
            },
        });
        let values = BTreeMap::from_iter(vec![(
            "ctx1".to_owned(),
            Arc::new(json!({
                "key3": 3,
            })),
        )]);
        let result = execute_jq(input.clone(), "$ctx1", &values).unwrap();
        assert_eq!(result, *(values["ctx1"]));
        let result = execute_jq(input, "$ctx1.key3", &values).unwrap();
        assert_eq!(result, 3);
    }

    #[test]
    fn test_lex_error() {
        let input = json!({});
        let values = BTreeMap::new();
        let error = execute_jq(input, "(", &values).expect_err("Should have failed to lex");
        let msg = format!("{error}");
        assert!(
            msg.contains("expected closing parenthesis"),
            "Expected lex error {msg}"
        );
    }

    #[test]
    fn test_parse_error() {
        let input = json!({});
        let values = BTreeMap::new();
        let error =
            execute_jq(input, "if false then .", &values).expect_err("Should have failed to parse");
        let msg = format!("{error}");
        assert!(
            msg.contains("expected else or end"),
            "Expected parse error {msg}"
        );
    }

    #[test]
    fn test_compile_error() {
        let input = json!({});
        let values = BTreeMap::new();
        let error = execute_jq(input, ".x | de", &values).expect_err("Should have failed to parse");
        let msg = format!("{error}");
        assert!(
            msg.contains("undefined filter"),
            "Expected compile error {msg}"
        );
    }

    #[test]
    fn test_multiple_compile_errors_separated() {
        let input = json!({});
        let values = BTreeMap::new();
        // Reference two undefined filters to trigger multiple compile errors
        let error =
            execute_jq(input, ".x | undefined1 | undefined2", &values).expect_err("Should fail");
        let msg = format!("{error}");
        // Multiple errors should be separated by ", "
        assert!(
            msg.contains(", "),
            "Multiple errors should be comma-separated: {msg}"
        );
        assert!(
            msg.matches("undefined").count() >= 2,
            "Should have at least 2 undefined errors: {msg}"
        );
    }

    #[test]
    fn runtime_error_is_returned_not_silently_dropped() {
        // Runtime errors must propagate; an earlier silent-drop bug surfaced
        // downstream as a confusing "expected X, got sequence" when the empty
        // result array was assigned into a proto field.
        let values = BTreeMap::new();
        let err = execute_jq(json!(1), ".x", &values)
            .expect_err("path access on a number must surface as Err");
        let msg = format!("{err}");
        assert!(
            msg.contains("jq runtime error"),
            "expected runtime error message, got: {msg}"
        );
    }

    #[test]
    fn test_undefined_variable_error() {
        let input = json!({"key": "value"});
        let mut values = BTreeMap::new();
        values.insert("defined_var".to_owned(), Arc::new(json!(123)));
        // Reference an undefined variable
        let error = execute_jq(input, "$undefined_var", &values).expect_err("Should fail");
        let msg = format!("{error}");
        assert!(
            msg.contains("undefined"),
            "Error should mention undefined: {msg}"
        );
    }

    // Round-trip tests exercise the `serde_json::Value` <-> `jaq_json::Val`
    // conversion via the identity filter `.`.

    fn assert_round_trip(v: serde_json::Value) {
        let actual = execute_jq(v.clone(), ".", &BTreeMap::new()).unwrap();
        assert_eq!(actual, v);
    }

    #[test]
    fn round_trip_integer() {
        for v in [
            json!(0),
            json!(42),
            json!(-1),
            json!(i64::MAX),
            json!(i64::MIN),
        ] {
            assert_round_trip(v);
        }
    }

    #[test]
    fn round_trip_large_integer() {
        // u64::MAX exceeds isize on 64-bit, exercising the BigInt branch.
        assert_round_trip(json!(u64::MAX));
    }

    #[test]
    fn round_trip_float() {
        for v in [json!(2.5_f64), json!(1.0e10_f64), json!(0.0_f64)] {
            assert_round_trip(v);
        }
    }

    #[test]
    fn round_trip_utf8_string() {
        for v in [json!("ascii"), json!("日本語"), json!("emoji 🦀✨")] {
            assert_round_trip(v);
        }
    }

    #[test]
    fn round_trip_nested() {
        assert_round_trip(json!({
            "a": [1, {"b": "c"}, null, true, [false, 2.5]],
            "z": {"nested": {"deep": [1, 2, 3]}},
        }));
    }

    #[test]
    fn round_trip_object_keys_preserved() {
        assert_round_trip(json!({"z": 1, "a": 2, "m": 3}));
    }

    // --- referenced variable extraction ---

    use super::{
        JqFilterCache, JqFilterCacheConfig, bound_variable_names, estimated_filter_bytes,
        referenced_variables,
    };
    use std::collections::BTreeSet;
    use std::time::Duration;

    fn refs(filter: &str) -> BTreeSet<&str> {
        referenced_variables(filter)
    }

    #[test]
    fn referenced_variables_extracts_names() {
        assert_eq!(refs("$a.k"), BTreeSet::from(["a"]));
        assert_eq!(refs("$a + $b_2"), BTreeSet::from(["a", "b_2"]));
        assert_eq!(refs("\"x\\($a)y\""), BTreeSet::from(["a"]));
        assert_eq!(refs("{$a, b: $c}"), BTreeSet::from(["a", "c"]));
        assert_eq!(refs("$__loc__"), BTreeSet::from(["__loc__"]));
        assert_eq!(refs(".a | .b"), BTreeSet::new());
        assert_eq!(refs("$"), BTreeSet::new());
        assert_eq!(refs("$1"), BTreeSet::new());
    }

    #[test]
    fn referenced_variables_over_approximates_string_literals() {
        // Binding a variable that is only mentioned in a string is harmless;
        // missing a real reference would not be.
        assert_eq!(refs("\"$x\""), BTreeSet::from(["x"]));
    }

    #[test]
    fn only_referenced_and_available_variables_are_bound() {
        let params = BTreeMap::from([
            ("a".to_owned(), Arc::new(json!(1))),
            ("big".to_owned(), Arc::new(json!({"huge": [1, 2, 3]}))),
            ("c".to_owned(), Arc::new(json!(3))),
        ]);
        assert_eq!(
            bound_variable_names("$a + $c + $missing", &params),
            vec!["a", "c"]
        );
        assert!(bound_variable_names(".", &params).is_empty());
    }

    // --- compile cache ---

    #[test]
    fn cached_filter_rebinds_values_on_each_call() {
        for v in 0..3 {
            let params = BTreeMap::from([("v".to_owned(), Arc::new(json!(v)))]);
            assert_eq!(
                execute_jq(json!(null), "$v * 10", &params).unwrap(),
                json!(v * 10)
            );
        }
    }

    #[test]
    fn cached_filter_is_keyed_by_bound_variable_set() {
        let filter = "[$b, $c]";
        let bc = BTreeMap::from([
            ("b".to_owned(), Arc::new(json!("b"))),
            ("c".to_owned(), Arc::new(json!("c"))),
        ]);
        assert_eq!(
            execute_jq(json!(null), filter, &bc).unwrap(),
            json!(["b", "c"])
        );
        // Adding a lower-sorted, unreferenced name must not shift positions.
        let mut abc = bc.clone();
        abc.insert("a".to_owned(), Arc::new(json!("a")));
        assert_eq!(
            execute_jq(json!(null), filter, &abc).unwrap(),
            json!(["b", "c"])
        );
        // Removing one referenced name must fail instead of reusing the cache.
        let only_c = BTreeMap::from([("c".to_owned(), Arc::new(json!("c")))]);
        assert!(execute_jq(json!(null), filter, &only_c).is_err());
    }

    #[test]
    fn compile_errors_are_not_cached() {
        let filter = "$late_output.value";
        let err = execute_jq(json!(null), filter, &BTreeMap::new())
            .expect_err("undefined variable must fail");
        assert!(format!("{err}").contains("undefined"), "{err}");
        let params = BTreeMap::from([("late_output".to_owned(), Arc::new(json!({"value": 7})))]);
        assert_eq!(execute_jq(json!(null), filter, &params).unwrap(), json!(7));
    }

    fn small_cache(entries: u64, time_to_idle: Option<Duration>) -> JqFilterCache {
        JqFilterCache::new(JqFilterCacheConfig {
            max_bytes: entries * u64::from(estimated_filter_bytes("$v + 00")),
            time_to_idle,
        })
    }

    #[test]
    fn filter_weight_grows_with_filter_length() {
        let short = estimated_filter_bytes(".a");
        let long = estimated_filter_bytes(&".a".repeat(1000));
        assert!(
            short >= 80 * 1024,
            "base cost of a compiled filter: {short}"
        );
        assert!(long > short);
        assert_eq!(estimated_filter_bytes(&"x".repeat(200_000_000)), u32::MAX);
    }

    #[test]
    fn cache_respects_memory_limit_and_keeps_results_correct() {
        let cache = small_cache(2, None);
        for round in 0..2 {
            for i in 10..20 {
                let params = BTreeMap::from([("v".to_owned(), Arc::new(json!(round)))]);
                let got = cache
                    .execute(&json!(null), &format!("$v + {i}"), &params)
                    .unwrap();
                assert_eq!(got, json!(round + i));
            }
        }
        cache.run_pending_tasks();
        assert!(cache.entry_count() <= 2, "entries: {}", cache.entry_count());
    }

    #[test]
    fn cache_evicts_idle_entries() {
        let cache = small_cache(10, Some(Duration::from_millis(50)));
        let params = BTreeMap::from([("v".to_owned(), Arc::new(json!(1)))]);
        assert_eq!(
            cache.execute(&json!(null), "$v + 1", &params).unwrap(),
            json!(2)
        );
        cache.run_pending_tasks();
        assert_eq!(cache.entry_count(), 1);
        std::thread::sleep(Duration::from_millis(200));
        cache.run_pending_tasks();
        assert_eq!(cache.entry_count(), 0);
        // An evicted filter is recompiled transparently.
        assert_eq!(
            cache.execute(&json!(null), "$v + 1", &params).unwrap(),
            json!(2)
        );
    }

    #[test]
    fn oversized_time_to_idle_does_not_panic() {
        let cache = small_cache(10, Some(Duration::MAX));
        assert_eq!(
            cache.execute(&json!(1), ". + 1", &BTreeMap::new()).unwrap(),
            json!(2)
        );
    }

    #[test]
    fn zero_memory_limit_disables_caching() {
        let cache = small_cache(0, None);
        let params = BTreeMap::new();
        for _ in 0..3 {
            assert_eq!(
                cache.execute(&json!(1), ". + 1", &params).unwrap(),
                json!(2)
            );
        }
        cache.run_pending_tasks();
        assert_eq!(cache.entry_count(), 0);
    }

    #[test]
    fn cache_reports_compile_errors_with_message() {
        let cache = small_cache(10, None);
        let err = cache
            .execute(&json!(null), "$nope", &BTreeMap::new())
            .expect_err("undefined variable must fail");
        assert!(format!("{err}").contains("undefined"), "{err}");
    }

    #[test]
    fn concurrent_evaluation_of_cached_filter_keeps_values_separate() {
        let handles: Vec<_> = (0..8)
            .map(|t| {
                std::thread::spawn(move || {
                    for i in 0..50 {
                        let v = t * 1000 + i;
                        let params = BTreeMap::from([("v".to_owned(), Arc::new(json!(v)))]);
                        let got = execute_jq(json!(1), ". + $v", &params).unwrap();
                        assert_eq!(got, json!(v + 1));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
    }

    // Manual timing check; run with:
    // cargo test --release -p command-utils jq_timing -- --ignored --nocapture
    #[test]
    #[ignore]
    fn jq_timing() {
        use std::time::Instant;
        fn big(kb: usize) -> serde_json::Value {
            let item = json!({"id": 1, "body": "x".repeat(60), "tags": ["a", "b"]});
            let n = kb * 1024 / serde_json::to_string(&item).unwrap().len();
            json!({"items": vec![item; n]})
        }
        fn time(label: &str, mut f: impl FnMut()) {
            let n = 300;
            let started = Instant::now();
            for _ in 0..n {
                f();
            }
            eprintln!(
                "{label:<40} {:>9.1} us/eval",
                started.elapsed().as_secs_f64() * 1e6 / n as f64
            );
        }
        let small = BTreeMap::from([("a".to_owned(), Arc::new(json!({"k": 1})))]);
        time("fixed cost ($a.k)", || {
            execute_jq(json!(1), "$a.k", &small).unwrap();
        });
        let mut with_big = small.clone();
        with_big.insert("big".to_owned(), Arc::new(big(200)));
        time("unreferenced 200KB variable", || {
            execute_jq(json!(1), "$a.k", &with_big).unwrap();
        });
        time("referenced 200KB variable", || {
            execute_jq(json!(1), "$big.items | length", &with_big).unwrap();
        });
    }

    fn assert_fromjson_finishes_with_error(input: serde_json::Value, filter: &str) {
        use std::time::{Duration, Instant};
        let started = Instant::now();
        let err = execute_jq(input, filter, &BTreeMap::new())
            .expect_err("invalid JSON must surface as a jq runtime error");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "fromjson must return instead of collecting an unbounded jaq stream"
        );
        let msg = format!("{err}");
        assert!(
            msg.contains("jq runtime error"),
            "expected runtime error message, got: {msg}"
        );
    }

    #[test]
    fn fromjson_valid_object_returns_value() {
        let input = json!({
            "content": { "text": "{\"category\":\"coding\"}" }
        });
        let result = execute_jq(input, ".content.text | fromjson", &BTreeMap::new()).unwrap();
        assert_eq!(result, json!({"category": "coding"}));
    }

    #[test]
    fn fromjson_ndjson_returns_array() {
        let input = json!("1 2 3");
        let result = execute_jq(input, "fromjson", &BTreeMap::new()).unwrap();
        assert_eq!(result, json!([1, 2, 3]));
    }

    // jaq-json's fromjson uses parse_many. An invalid leading token (markdown
    // fence, bare word) is peeked but not consumed, so the iterator never
    // ends unless the caller stops on the first error.
    #[test]
    fn fromjson_markdown_fence_returns_error_without_hanging() {
        let input = json!({
            "content": {
                "text": "```json\n{\"category\":\"coding\"}\n```"
            }
        });
        assert_fromjson_finishes_with_error(input, ".content.text | fromjson");
    }

    #[test]
    fn fromjson_invalid_leading_token_returns_error_without_hanging() {
        assert_fromjson_finishes_with_error(json!("hello"), "fromjson");
    }

    #[test]
    fn fromjson_valid_then_invalid_token_returns_error_without_hanging() {
        assert_fromjson_finishes_with_error(json!("{\"category\":\"coding\"}`"), "fromjson");
    }
}
