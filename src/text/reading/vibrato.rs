use anyhow::{Context, Result};
use std::fs::File;
use std::path::Path;

pub use vibrato::{Dictionary, Tokenizer};

/// Load a compiled MeCab/IPAdic dictionary from the specified path.
/// The dictionary must be pre-compiled (e.g. via `vibrato-tools compile`).
pub fn load_dictionary(dict_path: &Path) -> Result<Dictionary> {
    let reader = File::open(dict_path)
        .with_context(|| format!("failed to open vibrato dictionary: {:?}", dict_path))?;
    Dictionary::read(reader)
        .with_context(|| format!("failed to read vibrato dictionary: {:?}", dict_path))
}

/// Create a Tokenizer from a Dictionary with MeCab-compatible settings.
pub fn create_tokenizer(dict: Dictionary) -> Result<Tokenizer> {
    let tokenizer = Tokenizer::new(dict)
        .ignore_space(true)
        .map_err(|e| anyhow::anyhow!("failed to set ignore_space: {}", e))?
        .max_grouping_len(24);
    Ok(tokenizer)
}

/// Convert Japanese text to katakana pronunciation using feature field index 8 (発音).
/// Falls back to feature field index 7 (読み) then surface form when unavailable.
pub fn to_katakana_pronunciation(tokenizer: &Tokenizer, text: &str) -> Result<String> {
    to_katakana_by_field(tokenizer, text, 8, Some(7))
}

/// Convert Japanese text to katakana reading using feature field index 7 (読み).
/// Falls back to surface form when unavailable.
pub fn to_katakana_reading(tokenizer: &Tokenizer, text: &str) -> Result<String> {
    to_katakana_by_field(tokenizer, text, 7, None)
}

/// Extract katakana from a specific feature field index, with optional fallback field.
fn to_katakana_by_field(
    tokenizer: &Tokenizer,
    text: &str,
    primary_index: usize,
    fallback_index: Option<usize>,
) -> Result<String> {
    if text.is_empty() {
        return Ok(String::new());
    }

    let mut worker = tokenizer.new_worker();
    worker.reset_sentence(text);
    worker.tokenize();

    let mut result = String::with_capacity(text.len() * 2);
    for i in 0..worker.num_tokens() {
        let token = worker.token(i);
        let surface = token.surface();
        let feature = token.feature();

        let fields: Vec<&str> = feature.split(',').collect();
        let reading = extract_field(&fields, primary_index)
            .or_else(|| fallback_index.and_then(|idx| extract_field(&fields, idx)));

        match reading {
            Some(r) => result.push_str(r),
            None => result.push_str(surface),
        }
    }
    Ok(result)
}

/// Extract a non-empty, non-wildcard field value from feature fields.
fn extract_field<'a>(fields: &[&'a str], index: usize) -> Option<&'a str> {
    fields.get(index).and_then(|f| {
        let f = f.trim();
        if f.is_empty() || f == "*" {
            None
        } else {
            Some(f)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn get_tokenizer() -> Option<Arc<Tokenizer>> {
        let dict_path = std::env::var("VIBRATO_DICT_PATH").ok()?;
        let dict = load_dictionary(Path::new(&dict_path))
            .inspect_err(|e| eprintln!("error: {:?}", e))
            .ok()?;
        let tokenizer = create_tokenizer(dict)
            .inspect_err(|e| eprintln!("error: {:?}", e))
            .ok()?;
        Some(Arc::new(tokenizer))
    }

    #[test]
    fn test_load_dictionary_and_tokenizer() {
        let Some(_tokenizer) = get_tokenizer() else {
            eprintln!("VIBRATO_DICT_PATH not set, skipping");
            return;
        };
    }

    #[test]
    fn test_basic_kanji_reading() {
        let Some(tokenizer) = get_tokenizer() else {
            eprintln!("VIBRATO_DICT_PATH not set, skipping");
            return;
        };
        let result = to_katakana_reading(&tokenizer, "漢字").unwrap();
        assert_eq!(result, "カンジ");
    }

    #[test]
    fn test_pronunciation_ha_to_wa() {
        let Some(tokenizer) = get_tokenizer() else {
            eprintln!("VIBRATO_DICT_PATH not set, skipping");
            return;
        };
        // 発音フィールドでは助詞「は」が「ワ」になる
        let result = to_katakana_pronunciation(&tokenizer, "私は学生です").unwrap();
        assert!(
            result.contains("ワ"),
            "expected pronunciation to contain 'ワ' for particle は, got: {}",
            result
        );
    }

    #[test]
    fn test_empty_input() {
        let Some(tokenizer) = get_tokenizer() else {
            eprintln!("VIBRATO_DICT_PATH not set, skipping");
            return;
        };
        let result = to_katakana_reading(&tokenizer, "").unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn test_katakana_passthrough() {
        let Some(tokenizer) = get_tokenizer() else {
            eprintln!("VIBRATO_DICT_PATH not set, skipping");
            return;
        };
        let result = to_katakana_reading(&tokenizer, "カタカナ").unwrap();
        assert_eq!(result, "カタカナ");
    }

    #[test]
    fn test_pronunciation_complex_sentence() {
        let Some(tokenizer) = get_tokenizer() else {
            eprintln!("VIBRATO_DICT_PATH not set, skipping");
            return;
        };
        let result =
            to_katakana_pronunciation(&tokenizer, "今日の天気は晴れです").unwrap();
        assert!(!result.is_empty());
        // 「は」→「ワ」 in pronunciation
        assert!(
            result.contains("ワ"),
            "expected 'ワ' in pronunciation, got: {}",
            result
        );
    }
}
