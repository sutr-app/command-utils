use anyhow::{Result, anyhow};
use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};
use encoding_rs::{DecoderResult, Encoding, UTF_8, UTF_16BE, UTF_16LE};

/// read to end and detect char-encoding and decode to utf-8
/// (ignore unknown character)
pub fn encode_to_utf8<R>(input: &mut R) -> Result<String>
where
    R: std::io::Read,
{
    let mut reader: Vec<u8> = Vec::new();

    // read file
    input
        .read_to_end(&mut reader)
        .map_err(|e| anyhow!("Could not read file: {e}"))?;

    encode_to_utf8_raw(&reader)
}

/// detect char-encoding of the bytes and decode to utf-8
/// (ignore unknown character)
pub fn encode_to_utf8_raw(input: &[u8]) -> Result<String> {
    if let Some(text) = decode_with_bom(input) {
        return Ok(text);
    }
    // chardetng would also pick UTF-8 here, but only after scoring every candidate
    // encoding over the whole input.
    if let Ok(text) = std::str::from_utf8(input)
        && !text.is_ascii()
    {
        return Ok(text.to_owned());
    }
    if let Some(text) = decode_utf16_without_bom(input) {
        return Ok(text);
    }
    Ok(decode_ignoring_malformed(
        detect_byte_oriented_encoding(input),
        input,
    ))
}

/// chardetng intentionally leaves BOM sniffing to the caller.
fn decode_with_bom(input: &[u8]) -> Option<String> {
    let (encoding, bom_length) = Encoding::for_bom(input)?;
    Some(decode_ignoring_malformed(encoding, &input[bom_length..]))
}

/// UTF-16 without BOM is outside the scope of chardetng. Each char in U+0000-U+00FF puts
/// a NUL byte on the high-byte side of its code unit, whereas text in byte-oriented
/// encodings rarely contains NUL, so NULs are the evidence of UTF-16. Text with fewer such
/// chars (e.g. CJK only) is not supported and is left to the detector.
fn decode_utf16_without_bom(input: &[u8]) -> Option<String> {
    // UTF-16 text is practically never valid UTF-8 containing multi-byte sequences.
    if !input.contains(&0) || (!input.is_ascii() && std::str::from_utf8(input).is_ok()) {
        return None;
    }
    let encoding = guess_utf16_byte_order(input)?;
    // A trailing odd byte is a code unit cut at the end, dropped like other malformed input.
    let text = encoding
        .decode_without_bom_handling_and_without_replacement(&input[..input.len() / 2 * 2])?
        .into_owned();
    is_plausible_utf16(&text, input.is_ascii()).then_some(text)
}

/// A single NUL is too weak to tell UTF-16 from byte-oriented text ending with a stray NUL.
const MIN_HIGH_NUL_BYTES_FOR_UTF16: usize = 2;

fn guess_utf16_byte_order(input: &[u8]) -> Option<&'static Encoding> {
    let even = ByteStats::of(input.iter().step_by(2));
    let odd = ByteStats::of(input.iter().skip(1).step_by(2));
    // High bytes of UTF-16 code units concentrate on a few script blocks, so the side with
    // fewer distinct values is the high byte. NUL counts are not used first because
    // U+3000 or U+4E00 put NULs on the low-byte side.
    let little_endian =
        odd.distinct < even.distinct || (odd.distinct == even.distinct && odd.nul >= even.nul);
    let (encoding, high) = if little_endian {
        (UTF_16LE, odd)
    } else {
        (UTF_16BE, even)
    };
    (high.nul >= MIN_HIGH_NUL_BYTES_FOR_UTF16).then_some(encoding)
}

struct ByteStats {
    distinct: usize,
    nul: usize,
}

impl ByteStats {
    fn of<'a>(bytes: impl Iterator<Item = &'a u8>) -> Self {
        let mut seen = [false; 256];
        let mut nul = 0;
        for &b in bytes {
            seen[b as usize] = true;
            if b == 0 {
                nul += 1;
            }
        }
        ByteStats {
            distinct: seen.iter().filter(|&&s| s).count(),
            nul,
        }
    }
}

fn is_plausible_utf16(text: &str, input_is_ascii: bool) -> bool {
    // Stray NULs in byte-oriented text yield U+0000 or other controls when read as
    // UTF-16. Trailing U+0000 is allowed as a string terminator.
    let has_unexpected_control = text
        .trim_end_matches('\0')
        .chars()
        .any(|c| c.is_control() && !c.is_ascii_whitespace());
    if has_unexpected_control {
        return false;
    }
    // ASCII text separated by NULs (e.g. "a.txt\0b.txt\0") reads as UTF-16 whose code
    // units mostly pair two printable ASCII bytes, whereas UTF-16 made of ASCII-range bytes
    // only is Latin or Cyrillic text where such units are rare.
    !input_is_ascii || count_latin1(text) > count_printable_ascii_pairs(text)
}

fn count_latin1(text: &str) -> usize {
    text.chars()
        .filter(|c| ('\u{1}'..='\u{FF}').contains(c))
        .count()
}

fn count_printable_ascii_pairs(text: &str) -> usize {
    let printable_ascii = 0x20..=0x7E;
    text.chars()
        .map(u32::from)
        .filter(|&code| {
            printable_ascii.contains(&(code >> 8)) && printable_ascii.contains(&(code & 0xFF))
        })
        .count()
}

fn detect_byte_oriented_encoding(input: &[u8]) -> &'static Encoding {
    // ISO-2022-JP is denied by browsers only because decoded pages may run scripts,
    // which does not apply to plain text decoding here.
    let mut detector = EncodingDetector::new(Iso2022JpDetection::Allow);
    detector.feed(input, true);
    // no TLD hint: inputs are not always fetched from the web
    let detected = detector.guess(None, Utf8Detection::Allow);
    if detected != UTF_8 && looks_like_damaged_utf8(input) {
        UTF_8
    } else {
        detected
    }
}

/// Bytes decoded as UTF-8 that keep at least this many non-ASCII chars per malformed
/// sequence are treated as damaged UTF-8 rather than a legacy encoding. Legacy CJK text
/// decoded as UTF-8 yields below 0.4 such chars per malformed sequence for long text and
/// at most 3 for short snippets of a few chars.
const MIN_VALID_CHARS_PER_MALFORMED: usize = 5;

/// chardetng rejects UTF-8 on a single malformed byte or a sequence cut at the end of
/// the input, which would turn the whole text into mojibake of a legacy encoding.
fn looks_like_damaged_utf8(input: &[u8]) -> bool {
    let mut non_ascii = 0;
    let mut malformed = 0;
    let mut cut_at_end = false;
    let mut chunks = input.utf8_chunks().peekable();
    while let Some(chunk) = chunks.next() {
        non_ascii += chunk.valid().chars().filter(|c| !c.is_ascii()).count();
        let invalid = chunk.invalid();
        if invalid.is_empty() {
            continue;
        }
        // Cutting the input is not evidence of a legacy encoding.
        if chunks.peek().is_none() && is_incomplete_utf8(invalid) {
            cut_at_end = true;
        } else {
            malformed += 1;
        }
    }
    (malformed > 0 || cut_at_end)
        && non_ascii > 0
        && non_ascii >= malformed * MIN_VALID_CHARS_PER_MALFORMED
}

fn is_incomplete_utf8(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_err_and(|e| e.error_len().is_none())
}

/// Drop malformed byte sequences instead of emitting U+FFFD so that
/// the output does not contain replacement characters.
fn decode_ignoring_malformed(encoding: &'static Encoding, input: &[u8]) -> String {
    let mut decoder = encoding.new_decoder_without_bom_handling();
    // Decoding into a slice keeps the loop linear; the String-based API re-touches the
    // whole spare capacity on every call, i.e. on every malformed sequence.
    let capacity = decoder
        .max_utf8_buffer_length_without_replacement(input.len())
        .unwrap_or(input.len());
    let mut output = vec![0u8; capacity];
    let (mut read, mut written) = (0, 0);
    loop {
        let (result, r, w) = decoder.decode_to_utf8_without_replacement(
            &input[read..],
            &mut output[written..],
            true,
        );
        read += r;
        written += w;
        match result {
            DecoderResult::InputEmpty => break,
            DecoderResult::Malformed(_, _) => {}
            DecoderResult::OutputFull => output.resize(output.len() * 2, 0),
        }
    }
    output.truncate(written);
    // The worst-case capacity is up to 3x the input for legacy encodings.
    output.shrink_to_fit();
    String::from_utf8(output).expect("encoding_rs decoders emit valid UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    const JA_TEXT: &str = "吾輩は猫である。名前はまだ無い。どこで生れたかとんと見当がつかぬ。\
        何でも薄暗いじめじめした所でニャーニャー泣いていた事だけは記憶している。";
    const ZH_TEXT: &str = "中华人民共和国是工人阶级领导的、以工农联盟为基础的人民民主专政的社会主义国家。\
        社会主义制度是中华人民共和国的根本制度。";
    const KO_TEXT: &str = "대한민국은 민주공화국이다. 대한민국의 주권은 국민에게 있고, \
        모든 권력은 국민으로부터 나온다.";
    const RU_TEXT: &str = "Все люди рождаются свободными и равными в своем достоинстве и правах. \
        Они наделены разумом и совестью и должны поступать в отношении друг друга в духе братства.";
    const FR_TEXT: &str = "Tous les êtres humains naissent libres et égaux en dignité et en droits. \
        Ils sont doués de raison et de conscience et doivent agir les uns envers les autres \
        dans un esprit de fraternité. Ça être déjà très général à côté de la forêt.";

    fn encode(text: &str, encoding: &'static Encoding) -> Vec<u8> {
        let (bytes, _, had_errors) = encoding.encode(text);
        assert!(
            !had_errors,
            "test text must be encodable in {}",
            encoding.name()
        );
        bytes.into_owned()
    }

    fn encode_utf16(text: &str, little_endian: bool) -> Vec<u8> {
        text.encode_utf16()
            .flat_map(|unit| {
                if little_endian {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                }
            })
            .collect()
    }

    fn assert_decodes_from(text: &str, encoding: &'static Encoding) {
        assert_eq!(
            encode_to_utf8_raw(&encode(text, encoding)).unwrap(),
            text,
            "encoding: {}",
            encoding.name()
        );
    }

    /// `count` hiragana followed by one malformed byte
    fn utf8_with_one_malformed(count: usize) -> (String, Vec<u8>) {
        let text = "あ".repeat(count);
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0xFF);
        (text, bytes)
    }

    #[test]
    fn decodes_empty_input() {
        assert_eq!(encode_to_utf8_raw(&[]).unwrap(), "");
    }

    #[test]
    fn decodes_utf8() {
        assert_eq!(encode_to_utf8_raw(JA_TEXT.as_bytes()).unwrap(), JA_TEXT);
        assert_eq!(encode_to_utf8_raw(RU_TEXT.as_bytes()).unwrap(), RU_TEXT);
    }

    #[test]
    fn decodes_ascii() {
        let text = "Hello, world! {\"key\": [1, 2, 3]}\n";
        assert_eq!(encode_to_utf8_raw(text.as_bytes()).unwrap(), text);
    }

    #[test]
    fn decodes_legacy_encodings() {
        assert_decodes_from(JA_TEXT, encoding_rs::SHIFT_JIS);
        assert_decodes_from(JA_TEXT, encoding_rs::EUC_JP);
        assert_decodes_from(JA_TEXT, encoding_rs::ISO_2022_JP);
        assert_decodes_from(ZH_TEXT, encoding_rs::GBK);
        assert_decodes_from(KO_TEXT, encoding_rs::EUC_KR);
        assert_decodes_from(RU_TEXT, encoding_rs::WINDOWS_1251);
        assert_decodes_from(FR_TEXT, encoding_rs::WINDOWS_1252);
    }

    #[test]
    fn decodes_shift_jis_html_with_mostly_ascii_markup() {
        let html = format!(
            "<!DOCTYPE html><html><head><meta charset=\"Shift_JIS\">\
             <title>テスト</title></head><body><p>{JA_TEXT}</p></body></html>"
        );
        assert_decodes_from(&html, encoding_rs::SHIFT_JIS);
    }

    #[test]
    fn decodes_latin_text_ending_with_utf8_lead_byte() {
        assert_decodes_from(&format!("{FR_TEXT} Un café"), encoding_rs::WINDOWS_1252);
    }

    #[test]
    fn decodes_legacy_text_partially_valid_as_utf8() {
        // 3 chars valid as UTF-8 per malformed sequence when read as UTF-8
        assert_decodes_from("でニャー", encoding_rs::EUC_JP);
    }

    #[test]
    fn strips_utf8_bom() {
        let bytes = [&[0xEF, 0xBB, 0xBF][..], JA_TEXT.as_bytes()].concat();
        assert_eq!(encode_to_utf8_raw(&bytes).unwrap(), JA_TEXT);
    }

    #[test]
    fn decodes_utf16_with_bom() {
        let le = [&[0xFF, 0xFE][..], &encode_utf16(JA_TEXT, true)].concat();
        let be = [&[0xFE, 0xFF][..], &encode_utf16(JA_TEXT, false)].concat();
        assert_eq!(encode_to_utf8_raw(&le).unwrap(), JA_TEXT);
        assert_eq!(encode_to_utf8_raw(&be).unwrap(), JA_TEXT);
    }

    #[test]
    fn drops_lone_surrogate_in_utf16() {
        let bytes = [
            &[0xFF, 0xFE][..],
            &encode_utf16("前", true),
            &0xD800u16.to_le_bytes(),
            &encode_utf16("後", true),
        ]
        .concat();
        assert_eq!(encode_to_utf8_raw(&bytes).unwrap(), "前後");
    }

    #[test]
    fn decodes_utf16_without_bom() {
        let html = format!("<html><head><title>テスト</title></head><body>{JA_TEXT}</body></html>");
        let texts = [
            "Hello, world!\n",
            RU_TEXT,
            &html,
            // same distinct byte count on both sides: NUL counts decide the byte order
            "AA",
            // ideographic spaces put NUL bytes on the low-byte side of UTF-16 code units
            "　吾輩は猫である。　名前はまだ無い。\n　どこで生れたかとんと見当がつかぬ。\n",
        ];
        for text in texts {
            for little_endian in [true, false] {
                let bytes = encode_utf16(text, little_endian);
                assert_eq!(
                    encode_to_utf8_raw(&bytes).unwrap(),
                    text,
                    "little_endian: {little_endian}"
                );
            }
        }
    }

    #[test]
    fn decodes_damaged_utf16_without_bom() {
        let text = "Hello, world!\npage\x0Cbreak\n";
        for little_endian in [true, false] {
            let bytes = encode_utf16(text, little_endian);
            assert_eq!(encode_to_utf8_raw(&bytes).unwrap(), text);

            let truncated = &bytes[..bytes.len() - 1];
            assert_eq!(
                encode_to_utf8_raw(truncated).unwrap(),
                text.strip_suffix('\n').unwrap()
            );

            let nul_terminated = encode_utf16(&format!("{text}\0"), little_endian);
            assert_eq!(
                encode_to_utf8_raw(&nul_terminated).unwrap(),
                format!("{text}\0")
            );
        }
    }

    #[test]
    fn keeps_byte_oriented_text_with_stray_nul() {
        for text in [
            "hello world\0",
            "吾輩は猫である。\0",
            "texts\0\0\0",
            "ab\0cd\0",
            "foo\0bar\0",
            "key=value\0path=/usr/bin\0",
            "a.txt\0b.txt\0c.txt\0",
            "吾輩\0猫\0である\0",
        ] {
            assert_eq!(encode_to_utf8_raw(text.as_bytes()).unwrap(), text);
        }
        let mut bytes = encode(JA_TEXT, encoding_rs::SHIFT_JIS);
        bytes.extend_from_slice(&[0, 0]);
        assert_eq!(
            encode_to_utf8_raw(&bytes).unwrap(),
            format!("{JA_TEXT}\0\0")
        );
    }

    #[test]
    fn drops_malformed_bytes_in_utf8() {
        let mut bytes = JA_TEXT.as_bytes().to_vec();
        bytes.insert(JA_TEXT.find('。').unwrap(), 0xFF);
        bytes.push(0xE3); // truncated multi-byte sequence at end of input
        assert_eq!(encode_to_utf8_raw(&bytes).unwrap(), JA_TEXT);
    }

    #[test]
    fn keeps_mostly_ascii_utf8_truncated_mid_char() {
        for text in [
            "The café served crème brûlée and naïve coffee — truly “excellent”…",
            "<html><body><p>Ünïcödé</p></body></html>é",
        ] {
            let bytes = &text.as_bytes()[..text.len() - 1];
            let last_char_len = text.chars().last().unwrap().len_utf8();
            assert_eq!(
                encode_to_utf8_raw(bytes).unwrap(),
                text[..text.len() - last_char_len]
            );
        }
    }

    #[test]
    fn keeps_utf8_at_tolerance_boundary() {
        let (text, bytes) = utf8_with_one_malformed(MIN_VALID_CHARS_PER_MALFORMED);
        assert_eq!(encode_to_utf8_raw(&bytes).unwrap(), text);
    }

    #[test]
    fn treats_as_legacy_below_tolerance() {
        let (text, bytes) = utf8_with_one_malformed(MIN_VALID_CHARS_PER_MALFORMED - 1);
        assert_ne!(encode_to_utf8_raw(&bytes).unwrap(), text);
    }

    #[test]
    fn drops_malformed_sequences() {
        assert_eq!(decode_ignoring_malformed(UTF_8, b"a\xFFb\xFE\xE3"), "ab");
    }

    #[test]
    #[ignore = "timing test: run with `cargo test --release -- --ignored`"]
    fn drops_many_malformed_bytes_in_linear_time() {
        let bytes = vec![0xFF; 4 * 1024 * 1024];
        let started = std::time::Instant::now();
        let text = decode_ignoring_malformed(UTF_8, &bytes);
        let elapsed = started.elapsed();
        assert_eq!(text, "");
        assert!(
            elapsed < std::time::Duration::from_secs(1),
            "elapsed: {elapsed:?}"
        );
    }

    #[test]
    fn encode_to_utf8_reads_whole_input() {
        let bytes = encode(JA_TEXT, encoding_rs::SHIFT_JIS);
        let mut reader = std::io::Cursor::new(bytes);
        assert_eq!(encode_to_utf8(&mut reader).unwrap(), JA_TEXT);
    }

    #[test]
    fn encode_to_utf8_reports_read_error() {
        // reading a directory fails with EISDIR
        let mut dir = std::fs::File::open(std::env::temp_dir()).unwrap();
        assert!(encode_to_utf8(&mut dir).is_err());
    }
}
