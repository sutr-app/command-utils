use liquid_core::Result;
use liquid_core::Runtime;
use liquid_core::{Display_filter, Filter, FilterReflection, ParseFilter};
use liquid_core::{Value, ValueView};

#[derive(Clone, ParseFilter, FilterReflection)]
#[filter(
    name = "json_encode",
    description = "Converts any JSON-unsafe characters in a string into escaped characters.",
    parsed(JsonEncodeFilter)
)]
pub struct JsonEncode;

#[derive(Debug, Default, Display_filter)]
#[name = "json_encode"]
struct JsonEncodeFilter;

impl Filter for JsonEncodeFilter {
    fn evaluate(&self, input: &dyn ValueView, _runtime: &dyn Runtime) -> Result<Value> {
        if input.is_nil() {
            return Ok(Value::Nil);
        }

        let s = input.to_kstr();
        let json_escaped = serde_json::to_string(&s.to_string())
            .map_err(|e| liquid_core::Error::with_msg(format!("Malformed JSON string: {:?}", e)))?;
        let trimmed = json_escaped.trim_matches('"');
        Ok(Value::scalar(trimmed.to_string()))
    }
}

#[derive(Clone, ParseFilter, FilterReflection)]
#[filter(
    name = "json_decode",
    description = "Decodes a string that has been encoded as a JSON or by json_encode.",
    parsed(JsonDecodeFilter)
)]
pub struct JsonDecode;

#[derive(Debug, Default, Display_filter)]
#[name = "json_decode"]
struct JsonDecodeFilter;

impl Filter for JsonDecodeFilter {
    fn evaluate(&self, input: &dyn ValueView, _runtime: &dyn Runtime) -> Result<Value> {
        if input.is_nil() {
            return Ok(Value::Nil);
        }

        let s = input.to_kstr();
        // そのままserde_jsonでデコード
        let unescaped: String = serde_json::from_str(&s)
            .or_else(|_| serde_json::from_str(&format!("\"{}\"", s)))
            .map_err(|_| liquid_core::Error::with_msg("Malformed JSON string"))?;
        Ok(Value::scalar(unescaped))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_json_encode() {
        assert_eq!(
            liquid_core::call_filter!(JsonEncode, "foo bar").unwrap(),
            liquid_core::value!("foo bar")
        );
        assert_eq!(
            liquid_core::call_filter!(JsonEncode, "foo\"bar").unwrap(),
            liquid_core::value!("foo\\\"bar")
        );
        assert_eq!(
            liquid_core::call_filter!(JsonEncode, "foo\\bar").unwrap(),
            liquid_core::value!("foo\\\\bar")
        );
        assert_eq!(
            liquid_core::call_filter!(JsonEncode, r#"{"a":1,"b":2}"#).unwrap(),
            liquid_core::value!(r#"{\"a\":1,\"b\":2}"#)
        );
        assert_eq!(
            liquid_core::call_filter!(JsonEncode, "[1,2,3]").unwrap(),
            liquid_core::value!("[1,2,3]")
        );
        assert_eq!(
            liquid_core::call_filter!(JsonEncode, "foo\nbar").unwrap(),
            liquid_core::value!("foo\\nbar")
        );
        assert_eq!(
            liquid_core::call_filter!(JsonEncode, "foo\tbar").unwrap(),
            liquid_core::value!("foo\\tbar")
        );
        assert_eq!(
            liquid_core::call_filter!(JsonEncode, "foo\u{1}bar").unwrap(),
            liquid_core::value!("foo\\u0001bar")
        );
    }

    #[test]
    fn unit_json_decode() {
        assert_eq!(
            liquid_core::call_filter!(JsonDecode, "foo bar").unwrap(),
            liquid_core::value!("foo bar")
        );
        assert_eq!(
            liquid_core::call_filter!(JsonDecode, r#"foo\"bar"#).unwrap(),
            liquid_core::value!(r#"foo"bar"#)
        );
        assert_eq!(
            liquid_core::call_filter!(JsonDecode, r#"foo\\bar"#).unwrap(),
            liquid_core::value!(r#"foo\bar"#)
        );
        assert_eq!(
            liquid_core::call_filter!(JsonDecode, r#"{\"a\":1,\"b\":2}"#).unwrap(),
            liquid_core::value!(r#"{"a":1,"b":2}"#)
        );
        assert_eq!(
            liquid_core::call_filter!(JsonDecode, "[1,2,3]").unwrap(),
            liquid_core::value!("[1,2,3]")
        );
        // 改行
        assert_eq!(
            liquid_core::call_filter!(JsonDecode, "foo\\nbar").unwrap(),
            liquid_core::value!("foo\nbar")
        );
        // タブ
        assert_eq!(
            liquid_core::call_filter!(JsonDecode, "foo\\tbar").unwrap(),
            liquid_core::value!("foo\tbar")
        );
        // 制御文字 (0x01)
        assert_eq!(
            liquid_core::call_filter!(JsonDecode, "foo\\u0001bar").unwrap(),
            liquid_core::value!("foobar")
        );
    }
}
