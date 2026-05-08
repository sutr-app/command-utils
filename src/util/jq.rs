// reference: https://github.com/open-telemetry/weaver/blob/dc69528344429cc9ccc903fb3229ad051120c1a1/crates/weaver_forge/Cargo.toml
// SPDX-License-Identifier: Apache-2.0

//! Library to hide details of jaq from the rest of weaver.

use anyhow::{Result, anyhow};
use jaq_core::{
    Compiler, Ctx, Vars,
    data::JustLut,
    load::{Arena, File, Loader},
    unwrap_valr,
};
use jaq_json::{Num, Val};
use std::{collections::BTreeMap, sync::Arc};
type JqFileType = ();
type JqData = JustLut<Val>;

// use jaq_core::load::parse::Def,
// fn semconv_prelude() -> impl Iterator<Item = Def<&'static str>> {
//     jaq_core::load::parse(crate::SEMCONV_JQ, |p| p.defs())
//         .expect("BAD WEAVER BUILD - default JQ library failed to compile")
//         .into_iter()
// }

fn prepare_jq_context(
    params: &std::collections::BTreeMap<String, Arc<serde_json::Value>>,
) -> Result<(Vec<String>, Vec<Val>)> {
    params
        .iter()
        .map(|(k, v)| {
            let val = serde_json::from_value::<Val>((**v).clone())
                .map_err(|e| anyhow!("failed to convert jq context variable ${k}: {e}"))?;
            Ok((format!("${k}"), val))
        })
        .collect::<Result<Vec<_>>>()
        .map(|pairs| pairs.into_iter().unzip())
}

/// This is our single entry point for calling into the jaq library to run jq filters.
/// TODO: reuse the same jaq context for multiple calls.
pub fn execute_jq(
    // The JSON input to JQ.
    input: serde_json::Value,
    // The JQ filter to compile.
    filter_expr: &str,
    // Note: This will be exposed with `${key}` as the variable name.
    params: &BTreeMap<String, Arc<serde_json::Value>>,
) -> Result<serde_json::Value> {
    let loader = Loader::new(
        jaq_core::defs()
            .chain(jaq_std::defs())
            .chain(jaq_json::defs()),
    );
    let arena = Arena::default();
    let program: File<&str, JqFileType> = File {
        code: filter_expr,
        path: (), // ToDo - give this the weaver-config location.
    };

    // parse the filter
    let modules = loader
        .load(&arena, program)
        .map_err(load_errors)
        .map_err(|e| anyhow!(e))?;

    let (names, values) = prepare_jq_context(params)?;
    let funs = jaq_core::funs::<JqData>()
        .chain(jaq_std::funs::<JqData>())
        .chain(jaq_json::funs::<JqData>());
    // The `(name, args, native)` map shortens `&'static str` to a borrow that
    // matches `with_global_vars`'s shorter lifetime; without it the compiler
    // unifies on `'static` and rejects `names` (a local Vec).
    #[allow(clippy::map_identity)]
    let filter = Compiler::<&str, JqData>::default()
        .with_funs(funs.map(|(name, args, native)| (name, args, native)))
        .with_global_vars(names.iter().map(|s| s.as_str()))
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
    let ctx = Ctx::<JqData>::new(&filter.lut, Vars::new(values));

    let input_val = serde_json::from_value::<Val>(input)
        .map_err(|e| anyhow!("failed to convert jq input value: {e}"))?;
    let mut errs = Vec::new();
    let mut values = Vec::new();
    // `unwrap_valr` strips internal control-flow exceptions (tail call,
    // break) that must never reach the caller, leaving only proper errors.
    for r in filter.id.run((ctx, input_val)) {
        match unwrap_valr(r) {
            Ok(v) => values.push(val_to_json(v)?),
            Err(e) => errs.push(e),
        }
    }

    // Surface jaq runtime errors instead of silently dropping them.
    // Without this, a failing path access (e.g. `null | .x`) leaves `values` empty
    // and we return `Value::Array([])`, which then poisons downstream type checks
    // (proto schema parse, etc.) with a misleading "expected X, got sequence" error.
    if !errs.is_empty() {
        let msg = errs
            .iter()
            .map(|e| format!("{e}"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(anyhow!("jq runtime error for '{filter_expr}': {msg}"));
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
}
