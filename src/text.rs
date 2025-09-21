use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    hash::{DefaultHasher, Hasher},
};

use crate::util::datetime;

// 新規追加: 階層的チャンキング機能
pub mod chunking;

// for deserialize from env
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct SentenceSplitterCreator {
    pub max_buf_length: Option<usize>,
    pub delimiter_chars: Option<String>,
    pub force: Option<String>,
    pub parenthese_pairs: Option<String>,
}
impl SentenceSplitterCreator {
    // max input length for bert (max_position_embeddings)
    pub const DEFAULT_MAX_LENGTH: usize = 512;

    // 地の文だけをまとめる括弧。この前後で区切り間では区切らない。
    // ()[]は半角で使うことの方が多そうでプログラミングや顔文字など文以外でも使われる。
    // 正規化してもいいが半角のものとまざるし片方だけでも使われるケースがあるので危い。
    // これをまとめて役立つケースの方が少ない感じがするので一旦扱わない
    pub const PARENTHESE_PAIRS: [(char, char); 3] = [('「', '」'), ('『', '』'), ('【', '】')];

    // 正規化するよりは全角を特別扱いしたい"." (日本語文だと区切りで扱わない方がurlなどを過剰分割しないので良さそう)
    // があるので両方マッチしていいものは明示的に列挙する
    pub const DELIMITER_CHARS: [char; 7] = ['。', '．', '！', '？', '!', '?', '\n'];

    pub fn new(
        max_buf_length: Option<usize>,
        delimiter_chars: Option<String>,
        force: Option<String>,
        parenthese_pairs: Option<String>,
    ) -> Self {
        Self {
            max_buf_length,
            delimiter_chars,
            force,
            parenthese_pairs,
        }
    }
    pub fn new_by_env() -> Result<Self> {
        envy::prefixed("SENTENCE_SPLITTER_")
            .from_env::<SentenceSplitterCreator>()
            .context("cannot read SENTENCE_SPLITTER settings from env:")
    }
    pub fn create(&self) -> Result<SentenceSplitter> {
        let max_buf_length = self.max_buf_length.unwrap_or(Self::DEFAULT_MAX_LENGTH);
        let stop_chars = self
            .delimiter_chars
            .as_ref()
            .map(|s| s.chars().collect())
            .unwrap_or(Self::DELIMITER_CHARS.iter().cloned().collect());
        let force = self
            .force
            .as_ref()
            .map(|s| s.chars().collect())
            .unwrap_or_default();
        let parenthese_pairs: HashMap<char, char> = self
            .parenthese_pairs
            .as_ref()
            .map(|s| {
                let pairs: Vec<(char, char)> = s
                    .split(',')
                    .flat_map(|s| {
                        let mut chars = s.chars();
                        Some((chars.next()?, chars.next()?)) // ignore single char
                    })
                    .collect();
                pairs.into_iter().collect()
            })
            .unwrap_or(Self::PARENTHESE_PAIRS.iter().cloned().collect());
        let rev_parentheses = parenthese_pairs
            .iter()
            .map(|(a, b)| (*b, *a)) // iterate reverse
            .collect::<HashMap<char, char>>();

        Ok(SentenceSplitter {
            max_buf_length,
            delemeters: stop_chars,
            force,
            parenthese_pairs,
            rev_parentheses,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct SentenceSplitter {
    max_buf_length: usize,
    delemeters: HashSet<char>,
    force: HashSet<char>,
    parenthese_pairs: HashMap<char, char>,
    rev_parentheses: HashMap<char, char>,
}

impl SentenceSplitter {
    pub fn new_by_env() -> Result<Self> {
        let creator = SentenceSplitterCreator::new_by_env()?;
        creator.create()
    }

    pub fn split(&self, text: String) -> Vec<String> {
        let mut sentences: Vec<String> = vec![];
        let mut buf: Vec<char> = Vec::with_capacity(self.max_buf_length);
        let mut waiting_stack: Vec<&char> = vec![];

        for c in text.chars() {
            buf.push(c);

            if let Some(t) = self.parenthese_pairs.get(&c) {
                waiting_stack.push(t);
            } else if let Some(d) = waiting_stack.last() {
                if c == **d {
                    waiting_stack.pop();
                } else if self.force.contains(&c) {
                    sentences.push(buf.into_iter().collect());
                    buf = Vec::with_capacity(self.max_buf_length);
                    waiting_stack.clear();
                }
            } else if self.delemeters.contains(&c) {
                sentences.push(buf.into_iter().collect());
                buf = Vec::with_capacity(self.max_buf_length);
            }

            if buf.len() >= self.max_buf_length {
                sentences.push(buf.into_iter().collect());
                buf = Vec::with_capacity(self.max_buf_length);
                waiting_stack.clear()
            }
        }
        if !buf.is_empty() {
            sentences.push(buf.into_iter().collect());
        }
        sentences
    }

    //
    // XXX 最初の文がmaxより長い場合逆に切りつめられる。。。
    // (!!などの連続は扱いやすそうなのでどうにかならないか考える)
    pub fn split_r(&self, text: String) -> Vec<String> {
        let mut sentences: VecDeque<String> = VecDeque::new();
        let mut buf: VecDeque<char> = VecDeque::with_capacity(self.max_buf_length);
        let mut waiting_stack: Vec<&char> = vec![];

        // iterate reverse
        for c in text.chars().rev() {
            if let Some(t) = self.rev_parentheses.get(&c) {
                waiting_stack.push(t);
            } else if let Some(d) = waiting_stack.last() {
                if c == **d {
                    waiting_stack.pop();
                } else if self.force.contains(&c) {
                    sentences.push_front(buf.into_iter().collect());
                    buf = VecDeque::with_capacity(self.max_buf_length);
                    waiting_stack.clear();
                }
            } else if self.delemeters.contains(&c) && !buf.is_empty() {
                sentences.push_front(buf.into_iter().collect());
                buf = VecDeque::with_capacity(self.max_buf_length);
            }
            buf.push_front(c);

            if buf.len() >= self.max_buf_length {
                sentences.push_front(buf.into_iter().collect());
                buf = VecDeque::with_capacity(self.max_buf_length);
                waiting_stack.clear()
            }
        }
        if !buf.is_empty() {
            sentences.push_front(buf.into_iter().collect());
        }
        sentences.into()
    }

    pub fn split_with_div_regex<'a>(r: &Regex, text: &'a str) -> Vec<&'a str> {
        // parse timed text by token '<|time|>', and divide to vec
        // ex. "<|7.54|> All the time.<|12.34|><|12.98|> Interviews.<|15.50|><|16.04|> I'm your host.<|17.74|>" -> vec!["<|7.54|>"," All the time.","<|12.34|>","<|12.98|>"," Interviews.","<|15.50|>","<|16.04|>"," I'm your host.","<|17.74|>"]
        let mut divided = vec![];
        let mut prev = 0;
        for m in r.find_iter(text) {
            let (start, end) = (m.start(), m.end());
            if prev < start {
                divided.push(&text[prev..start]);
            }
            divided.push(&text[start..end]);
            prev = end;
        }
        if prev < text.len() {
            divided.push(&text[prev..]);
        }
        divided
    }
}
pub struct TextUtil {}

impl TextUtil {
    pub fn snake_to_camel(s: &str) -> String {
        s.split('_')
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().chain(c).collect(),
                }
            })
            .collect()
    }
    pub fn generate_random_key(prefix: Option<&String>) -> String {
        let mut hasher = DefaultHasher::default();
        hasher.write_i64(datetime::now_millis());
        hasher.write_i64(rand::random()); // random
        if let Some(p) = prefix {
            format!("{}_{:x}", p, hasher.finish())
        } else {
            format!("{:x}", hasher.finish())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // create test for SentenceSplitterConfig
    #[test]
    fn test_to_sentence_splitter() {
        let creator = SentenceSplitterCreator {
            max_buf_length: Some(100),
            delimiter_chars: Some("。,.．\n".to_string()),
            force: Some("".to_string()),
            parenthese_pairs: Some("「」,『』,(".to_string()),
        };
        let splitter = creator.create().unwrap();
        assert_eq!(splitter.max_buf_length, 100);
        assert_eq!(
            splitter.delemeters,
            HashSet::from_iter(vec!['。', ',', '.', '．', '\n'])
        );
        assert_eq!(splitter.force, HashSet::from_iter(Vec::new()));
        assert_eq!(
            splitter.parenthese_pairs,
            vec![('「', '」'), ('『', '』')]
                .into_iter()
                .collect::<HashMap<_, _>>()
        );
        assert_eq!(
            splitter.rev_parentheses,
            vec![('」', '「'), ('』', '『')].into_iter().collect()
        );
    }
    #[test]
    fn test_to_sentence_splitter_default() {
        let creator = SentenceSplitterCreator {
            max_buf_length: None,
            delimiter_chars: None,
            force: None,
            parenthese_pairs: None,
        };
        let splitter = creator.create().unwrap();
        assert_eq!(splitter.max_buf_length, 512);
        assert_eq!(
            splitter.delemeters,
            SentenceSplitterCreator::DELIMITER_CHARS
                .iter()
                .cloned()
                .collect()
        );
        assert_eq!(splitter.force, HashSet::new());
        assert_eq!(
            splitter.parenthese_pairs,
            SentenceSplitterCreator::PARENTHESE_PAIRS
                .iter()
                .cloned()
                .collect()
        );
    }

    #[test]
    fn test_split() {
        let splitter = SentenceSplitterCreator::new(None, None, None, None)
            .create()
            .unwrap();
        let text = "これはテストです。あれはテストではありません。".to_string();
        let sentences = splitter.split(text);
        assert_eq!(
            sentences,
            vec![
                "これはテストです。".to_string(),
                "あれはテストではありません。".to_string()
            ]
        );
    }

    #[test]
    fn test_split_with_single() {
        let stop_chars = "。";
        let splitter = SentenceSplitterCreator::new(None, Some(stop_chars.to_string()), None, None)
            .create()
            .unwrap();
        let text = "これはテストです。".to_string();
        let sentences = splitter.split(text);
        assert_eq!(sentences, vec!["これはテストです。".to_string()]);

        // // TODO
        // let text = "これはテストです!!あ。".to_string();
        // let sentences = splitter.split(text);
        // assert_eq!(
        //     sentences,
        //     vec!["これはテストです!!".to_string(), "あ。".to_string()]
        // );
    }
    #[test]
    fn test_split_with_force() {
        let force = "テ".to_string();
        let splitter = SentenceSplitterCreator::new(None, None, Some(force), None)
            .create()
            .unwrap();
        assert_eq!(splitter.force, HashSet::from_iter(vec!['テ']));
        let text = "「これはテストです。」".to_string();
        let sentences = splitter.split(text);
        // XXX forget parentheses after split.(divide last 」)
        assert_eq!(
            sentences,
            vec![
                "「これはテ".to_string(),
                "ストです。".to_string(),
                "」".to_string()
            ]
        );
    }
    #[test]
    fn test_split_with_force2() {
        let force = " ".to_string();
        let splitter =
            SentenceSplitterCreator::new(None, None, Some(force), Some("()".to_string()))
                .create()
                .unwrap();
        assert_eq!(splitter.force, HashSet::from_iter(vec![' ']));
        let text = "(This is a pen.)".to_string();
        let sentences = splitter.split(text);
        // XXX split at only first char
        assert_eq!(
            sentences,
            vec!["(This ".to_string(), "is a pen.)".to_string(),]
        );
    }
    #[test]
    fn test_split_with_parentheses() {
        let parentheses = "()".to_string();
        let splitter = SentenceSplitterCreator::new(None, None, None, Some(parentheses))
            .create()
            .unwrap();
        let text = "これはテスト(です。ああ)です。".to_string();
        let sentences = splitter.split(text);
        assert_eq!(
            sentences,
            vec!["これはテスト(です。ああ)です。".to_string()]
        );
    }
    #[test]
    fn test_split_with_max_buf_length() {
        let splitter = SentenceSplitterCreator::new(Some(2), None, None, None)
            .create()
            .unwrap();
        let text = "これはテストです。".to_string();
        let sentences = splitter.split(text);
        assert_eq!(sentences, vec!["これ", "はテ", "スト", "です", "。"]);
    }
    #[test]
    fn test_split_with_max_buf_length2() {
        let splitter = SentenceSplitterCreator::new(Some(5), None, None, None)
            .create()
            .unwrap();
        let text = "こ。れ。は。テストです。".to_string();
        let sentences = splitter.split(text);
        assert_eq!(sentences, vec!["こ。", "れ。", "は。", "テストです", "。"]);
    }
    // XXX now using and testing dividing timed text only
    #[test]
    fn test_split_with_div_regex() {
        let r = Regex::new(r"<\|([\d\.]+)\|>").unwrap();
        // for whisper v3 text
        let text = r#"<|7.54|> All the time.<|12.34|><|12.98|> Interviews.<|15.50|><|16.04|> I'm your host.<|17.74|><|19.46|> The idea<|24.38|><|24.38|> and applications.<|27.40|><|27.40|>"#;
        let expected = vec![
            "<|7.54|>",
            " All the time.",
            "<|12.34|>",
            "<|12.98|>",
            " Interviews.",
            "<|15.50|>",
            "<|16.04|>",
            " I'm your host.",
            "<|17.74|>",
            "<|19.46|>",
            " The idea",
            "<|24.38|>",
            "<|24.38|>",
            " and applications.",
            "<|27.40|>",
            "<|27.40|>",
        ];
        assert_eq!(SentenceSplitter::split_with_div_regex(&r, text), expected);

        let text = "<|20.00|> abcdefg<|29.80|>";
        let expected = vec!["<|20.00|>", " abcdefg", "<|29.80|>"];
        assert_eq!(SentenceSplitter::split_with_div_regex(&r, text), expected);

        let text = "<text<";
        let expected = vec!["<text<"];
        assert_eq!(SentenceSplitter::split_with_div_regex(&r, text), expected);

        let text = "<|00.80|>";
        let expected = vec!["<|00.80|>"];
        assert_eq!(SentenceSplitter::split_with_div_regex(&r, text), expected);
    }
    #[test]
    fn test_snake_to_camel() {
        assert_eq!(TextUtil::snake_to_camel("snake_to_camel"), "SnakeToCamel");
        assert_eq!(TextUtil::snake_to_camel("snake_to_camel_"), "SnakeToCamel");
        assert_eq!(TextUtil::snake_to_camel("_snake_to_camel"), "SnakeToCamel");
        assert_eq!(TextUtil::snake_to_camel("snakeToCamel"), "SnakeToCamel");
        assert_eq!(TextUtil::snake_to_camel("snake?"), "Snake?");
        assert_eq!(TextUtil::snake_to_camel("SNAKE_TO_CAMEL"), "SNAKETOCAMEL"); // XXX
    }
}
