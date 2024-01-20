use regex::Regex;
use std::collections::{HashMap, HashSet, VecDeque};

pub struct SentenceSplitter {
    max_buf_length: usize,
    stop_chars: HashSet<char>,
    force: HashSet<char>,
    parentheses: HashMap<char, char>,
    rev_parentheses: HashMap<char, char>,
}

impl SentenceSplitter {
    // max input length for bert (max_position_embeddings)
    pub const DEFAULT_MAX_LENGTH: usize = 512;

    // 地の文だけをまとめる括弧。この前後で区切り間では区切らない。
    // ()[]は半角で使うことの方が多そうでプログラミングや顔文字など文以外でも使われる。
    // 正規化してもいいが半角のものとまざるし片方だけでも使われるケースがあるので危い。
    // これをまとめて役立つケースの方が少ない感じがするので一旦扱わない
    pub const PARENTHESES: [(char, char); 3] = [('「', '」'), ('『', '』'), ('【', '】')];

    // 正規化するよりは全角を特別扱いしたい"." (日本語文だと区切りで扱わない方がurlなどを過剰分割しないので良さそう)
    // があるので両方マッチしていいものは明示的に列挙する
    pub const STOP_CHARS: [char; 7] = ['。', '．', '！', '？', '!', '?', '\n'];

    pub fn new(
        max_buf_length: Option<usize>,
        stop_chars: Option<HashSet<char>>,
        force: Option<HashSet<char>>,
        parentheses: Option<HashMap<char, char>>,
    ) -> Self {
        let max_buf_length = max_buf_length.unwrap_or(Self::DEFAULT_MAX_LENGTH);
        let mut stop_chars = stop_chars.unwrap_or(Self::STOP_CHARS.iter().cloned().collect());
        let force = force
            .map(|f| {
                stop_chars.extend(f.iter().cloned());
                f
            })
            .unwrap_or_default();
        let parentheses = parentheses.unwrap_or(Self::PARENTHESES.iter().cloned().collect());
        // reverse pair of parentheses for reverse iteration
        let rev_parentheses = parentheses
            .iter()
            .map(|(a, b)| (*b, *a)) // iterate reverse
            .collect::<HashMap<char, char>>();
        SentenceSplitter {
            max_buf_length,
            stop_chars,
            force,
            parentheses,
            rev_parentheses,
        }
    }

    pub fn split(&self, text: String) -> Vec<String> {
        let mut sentences: Vec<String> = vec![];
        let mut buf: Vec<char> = Vec::with_capacity(self.max_buf_length);
        let mut waiting_stack: Vec<&char> = vec![];

        for c in text.chars() {
            buf.push(c);

            if let Some(t) = self.parentheses.get(&c) {
                waiting_stack.push(t);
            } else if let Some(d) = waiting_stack.last() {
                if c == **d {
                    waiting_stack.pop();
                } else if self.force.contains(&c) {
                    sentences.push(buf.into_iter().collect());
                    buf = Vec::with_capacity(self.max_buf_length);
                    waiting_stack.clear();
                }
            } else if self.stop_chars.contains(&c) && !buf.is_empty() {
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
            } else if self.stop_chars.contains(&c) && !buf.is_empty() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split() {
        let splitter = SentenceSplitter::new(None, None, None, None);
        let text = "これはテストです。".to_string();
        let sentences = splitter.split(text);
        assert_eq!(sentences, vec!["これはテストです。".to_string()]);
    }

    #[test]
    fn test_split_with_stop_chars() {
        let mut stop_chars = HashSet::new();
        stop_chars.insert('。');
        let splitter = SentenceSplitter::new(None, Some(stop_chars), None, None);
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
        let mut force = HashSet::new();
        force.insert('テ');
        let splitter = SentenceSplitter::new(None, None, Some(force), None);
        let text = "これはテストです。".to_string();
        let sentences = splitter.split(text);
        assert_eq!(
            sentences,
            vec!["これはテ".to_string(), "ストです。".to_string()]
        );
    }
    #[test]
    fn test_split_with_parentheses() {
        let mut parentheses = HashMap::new();
        parentheses.insert('(', ')');
        let splitter = SentenceSplitter::new(None, None, None, Some(parentheses));
        let text = "これはテスト(です。ああ)です。".to_string();
        let sentences = splitter.split(text);
        assert_eq!(
            sentences,
            vec!["これはテスト(です。ああ)です。".to_string()]
        );
    }
    #[test]
    fn test_split_with_max_buf_length() {
        let splitter = SentenceSplitter::new(Some(2), None, None, None);
        let text = "これはテストです。".to_string();
        let sentences = splitter.split(text);
        assert_eq!(sentences, vec!["これ", "はテ", "スト", "です", "。"]);
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

        let text = format!("<|20.00|> abcdefg<|29.80|>");
        let expected = vec!["<|20.00|>", " abcdefg", "<|29.80|>"];
        assert_eq!(
            SentenceSplitter::split_with_div_regex(&r, text.as_str()),
            expected
        );

        let text = format!("<text<");
        let expected = vec!["<text<"];
        assert_eq!(
            SentenceSplitter::split_with_div_regex(&r, text.as_str()),
            expected
        );

        let text = format!("<|00.80|>");
        let expected = vec!["<|00.80|>"];
        assert_eq!(
            SentenceSplitter::split_with_div_regex(&r, text.as_str()),
            expected
        );
    }
}
