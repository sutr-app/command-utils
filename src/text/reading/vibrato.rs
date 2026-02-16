use anyhow::{Context, Result};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

pub use vibrato::{Dictionary, Tokenizer};

/// Dictionary type determines feature field layout for reading/pronunciation extraction.
/// vibrato's API is format-agnostic; field semantics depend on the dictionary used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DictType {
    /// IPAdic/NAIST-jdic (9 fields): reading=field[7] (katakana), pronunciation=field[8] (katakana)
    IpaDic,
    /// UniDic (17 or 26+ fields): reading=field[6] (katakana), pronunciation=field[9] (katakana)
    UniDic,
    /// JumanDic (7 fields): reading=field[5] (hiragana), no pronunciation field
    JumanDic,
}

impl DictType {
    fn reading_index(self) -> usize {
        match self {
            Self::IpaDic => 7,
            Self::UniDic => 6,
            Self::JumanDic => 5,
        }
    }

    fn pronunciation_index(self) -> Option<usize> {
        match self {
            Self::IpaDic => Some(8),
            Self::UniDic => Some(9),
            // JumanDic has no separate pronunciation field
            Self::JumanDic => None,
        }
    }

    fn is_hiragana_reading(self) -> bool {
        matches!(self, Self::JumanDic)
    }
}

/// Load a compiled vibrato dictionary from the specified path.
/// Automatically detects and decompresses zstd-compressed dictionaries (.zst/.zstd extension).
pub fn load_dictionary(dict_path: &Path) -> Result<Dictionary> {
    let file = File::open(dict_path)
        .with_context(|| format!("failed to open vibrato dictionary: {:?}", dict_path))?;

    let is_zstd = dict_path
        .extension()
        .is_some_and(|ext| ext == "zst" || ext == "zstd");

    if is_zstd {
        let decoder = zstd::Decoder::new(BufReader::new(file))
            .with_context(|| format!("failed to create zstd decoder for {:?}", dict_path))?;
        Dictionary::read(decoder)
            .with_context(|| format!("failed to read vibrato dictionary from {:?}", dict_path))
    } else {
        Dictionary::read(BufReader::new(file))
            .with_context(|| format!("failed to read vibrato dictionary from {:?}", dict_path))
    }
}

/// Create a Tokenizer from a Dictionary with MeCab-compatible settings.
pub fn create_tokenizer(dict: Dictionary) -> Result<Tokenizer> {
    let tokenizer = Tokenizer::new(dict)
        .ignore_space(true)
        .map_err(|e| anyhow::anyhow!("failed to set ignore_space: {}", e))?
        .max_grouping_len(24);
    Ok(tokenizer)
}

/// Detect dictionary type by probing feature fields of a known token.
///
/// Heuristic based on field count:
/// - JumanDic: 7 fields, reading in hiragana at field[5]
/// - IPAdic: ~9 fields, reading in katakana at field[7]
/// - UniDic: 17+ fields, reading in katakana at field[6]
pub fn detect_dict_type(tokenizer: &Tokenizer) -> DictType {
    let mut worker = tokenizer.new_worker();
    worker.reset_sentence("の");
    worker.tokenize();
    if worker.num_tokens() > 0 {
        let field_count = worker.token(0).feature().split(',').count();
        if field_count >= 13 {
            return DictType::UniDic;
        }
        if field_count <= 7 {
            return DictType::JumanDic;
        }
    }
    DictType::IpaDic
}

/// Convert Japanese text to katakana pronunciation.
/// Uses the pronunciation field (IPAdic: field[8], UniDic: field[9]).
/// Returns an error for JumanDic which has no pronunciation field.
pub fn to_katakana_pronunciation(
    tokenizer: &Tokenizer,
    dict_type: DictType,
    text: &str,
) -> Result<String> {
    let primary = dict_type
        .pronunciation_index()
        .ok_or_else(|| anyhow::anyhow!("pronunciation is not available for {:?}", dict_type))?;
    to_katakana_by_field(tokenizer, text, primary, None, false)
}

/// Convert Japanese text to katakana reading.
/// Uses the reading field (IPAdic: field[7], UniDic: field[6], JumanDic: field[5]).
/// JumanDic readings are in hiragana and automatically converted to katakana.
pub fn to_katakana_reading(
    tokenizer: &Tokenizer,
    dict_type: DictType,
    text: &str,
) -> Result<String> {
    let primary = dict_type.reading_index();
    to_katakana_by_field(
        tokenizer,
        text,
        primary,
        None,
        dict_type.is_hiragana_reading(),
    )
}

/// Extract katakana from a specific feature field index, with optional fallback field.
/// When `convert_hiragana` is true, converts hiragana readings to katakana (for JumanDic).
fn to_katakana_by_field(
    tokenizer: &Tokenizer,
    text: &str,
    primary_index: usize,
    fallback_index: Option<usize>,
    convert_hiragana: bool,
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
            Some(r) if convert_hiragana => result.push_str(&hiragana_to_katakana(r)),
            Some(r) => result.push_str(r),
            None => result.push_str(surface),
        }
    }
    Ok(result)
}

/// Convert hiragana characters to katakana. Non-hiragana characters pass through unchanged.
fn hiragana_to_katakana(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{3041}'..='\u{3096}' => char::from_u32(c as u32 + 0x60).unwrap_or(c),
            _ => c,
        })
        .collect()
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

    fn get_tokenizer() -> Option<(Tokenizer, DictType)> {
        let dict_path = std::env::var("VIBRATO_DICT_PATH").ok()?;
        let dict = load_dictionary(Path::new(&dict_path))
            .inspect_err(|e| eprintln!("error: {:?}", e))
            .ok()?;
        let tokenizer = create_tokenizer(dict)
            .inspect_err(|e| eprintln!("error: {:?}", e))
            .ok()?;
        let dict_type = detect_dict_type(&tokenizer);
        eprintln!("detected dict_type: {:?}", dict_type);
        Some((tokenizer, dict_type))
    }

    #[test]
    fn test_hiragana_to_katakana() {
        assert_eq!(hiragana_to_katakana("あいうえお"), "アイウエオ");
        assert_eq!(hiragana_to_katakana("かきくけこ"), "カキクケコ");
        assert_eq!(hiragana_to_katakana("ぁぃぅぇぉ"), "ァィゥェォ");
        // katakana and ASCII pass through unchanged
        assert_eq!(hiragana_to_katakana("カタカナ"), "カタカナ");
        assert_eq!(hiragana_to_katakana("abc"), "abc");
        // mixed
        assert_eq!(hiragana_to_katakana("あaカ"), "アaカ");
        assert_eq!(hiragana_to_katakana(""), "");
    }

    #[test]
    fn test_extract_field() {
        let fields = vec!["名詞", "一般", "*", "*", "*", "*", "東京", "トウキョウ", "トーキョー"];
        assert_eq!(extract_field(&fields, 7), Some("トウキョウ"));
        assert_eq!(extract_field(&fields, 2), None); // "*" treated as empty
        assert_eq!(extract_field(&fields, 99), None); // out of bounds
        assert_eq!(extract_field(&fields, 0), Some("名詞"));

        let empty_fields: Vec<&str> = vec![];
        assert_eq!(extract_field(&empty_fields, 0), None);

        let whitespace = vec!["  ", " foo "];
        assert_eq!(extract_field(&whitespace, 0), None); // whitespace-only
        assert_eq!(extract_field(&whitespace, 1), Some("foo")); // trimmed
    }

    #[test]
    fn test_load_dictionary_and_tokenizer() {
        let Some(_) = get_tokenizer() else {
            eprintln!("VIBRATO_DICT_PATH not set, skipping");
            return;
        };
    }

    #[test]
    fn test_basic_kanji_reading() {
        let Some((tokenizer, dict_type)) = get_tokenizer() else {
            eprintln!("VIBRATO_DICT_PATH not set, skipping");
            return;
        };
        let result = to_katakana_reading(&tokenizer, dict_type, "漢字").unwrap();
        assert_eq!(result, "カンジ");
    }

    #[test]
    fn test_pronunciation_ha_to_wa() {
        let Some((tokenizer, dict_type)) = get_tokenizer() else {
            eprintln!("VIBRATO_DICT_PATH not set, skipping");
            return;
        };
        if dict_type.pronunciation_index().is_none() {
            assert!(to_katakana_pronunciation(&tokenizer, dict_type, "僕は学生です").is_err());
            return;
        }
        let result = to_katakana_pronunciation(&tokenizer, dict_type, "僕は学生です").unwrap();
        assert!(
            result == "ボクワガクセーデス" || result == "ボクワガクセイデス",
            "expected pronunciation 'ボクワガクセーデス' for 僕は学生です, got: {}",
            result
        );
    }

    #[test]
    fn test_empty_input() {
        let Some((tokenizer, dict_type)) = get_tokenizer() else {
            eprintln!("VIBRATO_DICT_PATH not set, skipping");
            return;
        };
        let result = to_katakana_reading(&tokenizer, dict_type, "").unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn test_katakana_passthrough() {
        let Some((tokenizer, dict_type)) = get_tokenizer() else {
            eprintln!("VIBRATO_DICT_PATH not set, skipping");
            return;
        };
        let result = to_katakana_reading(&tokenizer, dict_type, "カタカナ").unwrap();
        assert_eq!(result, "カタカナ");
    }

    #[test]
    fn test_pronunciation_complex_sentence() {
        let Some((tokenizer, dict_type)) = get_tokenizer() else {
            eprintln!("VIBRATO_DICT_PATH not set, skipping");
            return;
        };
        if dict_type.pronunciation_index().is_none() {
            assert!(
                to_katakana_pronunciation(&tokenizer, dict_type, "今日の天気は晴れです").is_err()
            );
            return;
        }
        let result =
            to_katakana_pronunciation(&tokenizer, dict_type, "今日の天気は晴れです").unwrap();
        assert!(!result.is_empty());
        assert!(
            result.contains("ワ"),
            "expected 'ワ' in reading, got: {}",
            result
        );
    }
}
