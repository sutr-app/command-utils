use anyhow::{Context, Result};
use std::path::Path;

pub use sudachi::analysis::Mode;
pub use sudachi::dic::dictionary::JapaneseDictionary;

use sudachi::analysis::Tokenize;
use sudachi::analysis::stateless_tokenizer::StatelessTokenizer;
use sudachi::config::Config;

/// Load a JapaneseDictionary from the specified dictionary file path.
/// Uses sudachi's built-in resources (char.def, unk.def, etc.) so only the .dic file is needed.
pub fn load_dictionary(dict_path: &Path) -> Result<JapaneseDictionary> {
    let mut config = Config::new_embedded()
        .with_context(|| "failed to create embedded sudachi config")?;
    config.system_dict = Some(dict_path.to_path_buf());
    JapaneseDictionary::from_cfg(&config)
        .with_context(|| format!("failed to load sudachi dictionary from {:?}", dict_path))
}

/// Convert kanji-mixed Japanese text to katakana reading.
/// Uses Mode::C (longest unit) for natural TTS reading.
/// OOV morphemes fall back to surface form.
pub fn to_katakana_reading(dict: &JapaneseDictionary, text: &str) -> Result<String> {
    to_katakana_reading_with_mode(dict, text, Mode::C)
}

/// Same as `to_katakana_reading` with explicit mode selection.
pub fn to_katakana_reading_with_mode(
    dict: &JapaneseDictionary,
    text: &str,
    mode: Mode,
) -> Result<String> {
    if text.is_empty() {
        return Ok(String::new());
    }

    let tokenizer = StatelessTokenizer::new(dict);
    let morphemes = tokenizer
        .tokenize(text, mode, false)
        .with_context(|| format!("failed to tokenize: {:?}", text))?;

    let mut result = String::with_capacity(text.len() * 2);
    for i in 0..morphemes.len() {
        let morpheme = morphemes.get(i);
        let reading = morpheme.reading_form();
        if reading.is_empty() {
            // OOV: fall back to surface form
            result.push_str(&morpheme.surface());
        } else {
            result.push_str(reading);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_dict() -> Option<JapaneseDictionary> {
        let dict_path = std::env::var("SUDACHI_DICT_PATH").ok()?;
        let dict = load_dictionary(Path::new(&dict_path))
            .inspect_err(|e| eprintln!("error: {:?}", e))
            .ok()?;
        Some(dict)
    }

    #[test]
    fn test_load_dictionary() {
        let Some(_dict) = get_dict() else {
            eprintln!("SUDACHI_DICT_PATH not set, skipping");
            return;
        };
    }

    #[test]
    fn test_basic_kanji_reading() {
        let Some(dict) = get_dict() else {
            eprintln!("SUDACHI_DICT_PATH not set, skipping");
            return;
        };
        let result = to_katakana_reading(&dict, "漢字").unwrap();
        assert_eq!(result, "カンジ");
    }

    #[test]
    fn test_basic_kanji_reading2() {
        let Some(dict) = get_dict() else {
            eprintln!("SUDACHI_DICT_PATH not set, skipping");
            return;
        };
        let text = "今日の天気は晴、ところにより一時雨。 降水確率は40パーセントです";
        let result = to_katakana_reading(&dict, text).unwrap();
        println!("result:{}", result);
        // Verify the output is non-empty katakana
        assert!(!result.is_empty());
        assert!(result.contains("キョウ"));
        assert!(result.contains("テンキ"));
        assert!(result.contains("コウスイカクリツ"));
    }

    #[test]
    fn test_mixed_text_reading() {
        let Some(dict) = get_dict() else {
            eprintln!("SUDACHI_DICT_PATH not set, skipping");
            return;
        };
        let result = to_katakana_reading(&dict, "東京都に行く").unwrap();
        // Mode::C groups longer units
        assert!(!result.is_empty());
        // Should contain katakana
        assert!(result.chars().all(|c| is_katakana_or_punctuation(c)));
    }

    #[test]
    fn test_empty_input() {
        let Some(dict) = get_dict() else {
            eprintln!("SUDACHI_DICT_PATH not set, skipping");
            return;
        };
        let result = to_katakana_reading(&dict, "").unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn test_katakana_passthrough() {
        let Some(dict) = get_dict() else {
            eprintln!("SUDACHI_DICT_PATH not set, skipping");
            return;
        };
        let result = to_katakana_reading(&dict, "カタカナ").unwrap();
        assert_eq!(result, "カタカナ");
    }

    #[test]
    fn test_mode_a_finer_granularity() {
        let Some(dict) = get_dict() else {
            eprintln!("SUDACHI_DICT_PATH not set, skipping");
            return;
        };
        let result_c = to_katakana_reading_with_mode(&dict, "国会議事堂", Mode::C).unwrap();
        let result_a = to_katakana_reading_with_mode(&dict, "国会議事堂", Mode::A).unwrap();
        // Both should produce valid katakana readings (content may differ but reading is same)
        assert!(!result_c.is_empty());
        assert!(!result_a.is_empty());
    }

    fn is_katakana_or_punctuation(c: char) -> bool {
        matches!(c, '\u{30A0}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}' | '\u{FF65}'..='\u{FF9F}'
            | '、' | '。' | '！' | '？' | ' ' | '　')
    }
}
