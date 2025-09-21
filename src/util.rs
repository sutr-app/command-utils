pub mod encoding;
pub mod id_generator;
pub mod jq;
pub mod json_schema;
pub mod liquid;
pub mod scoped_cache;
pub mod shutdown;
pub mod stack;
pub mod tracing;

pub mod result {
    use anyhow::Result;
    // use async_trait::async_trait;
    // use std::pin::Pin;

    // use encoding::all;
    use futures::future::BoxFuture;
    #[deprecated(note = "use `and_then` instead")]
    pub trait FlatMap<T, U, F: FnOnce(T) -> Result<U>> {
        fn flat_map(self, op: F) -> Result<U>;
    }
    #[allow(deprecated)]
    impl<T, U, F: FnOnce(T) -> Result<U>> FlatMap<T, U, F> for Result<T> {
        #[inline]
        fn flat_map(self, op: F) -> Result<U> {
            match self {
                Ok(r) => op(r),
                Err(e) => Err(e),
            }
        }
    }

    #[deprecated(note = "use `inspect` instead")]
    pub trait Tap<T, E, F> {
        fn tap(self, f: F) -> Result<T, E>;
    }

    #[allow(deprecated)]
    impl<T, E, F> Tap<T, E, F> for Result<T, E>
    where
        F: FnOnce(&T),
    {
        #[inline]
        fn tap(self, f: F) -> Result<T, E> {
            match self {
                Ok(r) => {
                    f(&r);
                    Ok(r)
                }
                Err(e) => Err(e),
            }
        }
    }

    #[deprecated(note = "use `inspect_err` instead")]
    pub trait TapErr<T, E, F> {
        fn tap_err(self, f: F) -> Result<T, E>;
    }

    #[allow(deprecated)]
    impl<T, E, F> TapErr<T, E, F> for Result<T, E>
    where
        F: FnOnce(&E),
    {
        #[inline]
        fn tap_err(self, f: F) -> Result<T, E> {
            match self {
                Ok(r) => Ok(r),
                Err(e) => {
                    f(&e);
                    Err(e)
                }
            }
        }
    }

    pub trait Flatten<T> {
        fn flatten(self) -> Result<T>;
    }
    impl<T> Flatten<T> for Result<Result<T>> {
        #[inline]
        fn flatten(self) -> Result<T> {
            match self {
                Ok(r) => r,
                Err(e) => Err(e),
            }
        }
    }
    // #[async_trait]
    // pub trait AsyncFlatMap<T, U, E, F> {
    //     async fn flat_map_async(self, op: F) -> Result<U, E>;
    // }
    //
    // #[async_trait]
    // impl<'a, T, U, E, F> AsyncFlatMap<T, U, E, F> for Result<T, E>
    // where
    //     Self: Send,
    //     T: Send + Sized + 'static,
    //     U: Send + Sized + 'a,
    //     E: Send + Sized + 'static,
    //     F: FnOnce(T) -> dyn Future<Output = Result<U, E>>,
    // {
    //     #[inline]
    //     async fn flat_map_async(self, op: F) -> Result<U, E> {
    //         match self {
    //             Ok(r) => op(r).await,
    //             Err(e) => Err(e),
    //         }
    //     }
    // }

    pub trait AsyncFlatMap<T, U, E, F> {
        fn flat_map_async(self, op: F) -> BoxFuture<'static, Result<U, E>>;
    }

    impl<T, U, E, F> AsyncFlatMap<T, U, E, F> for Result<T, E>
    where
        T: Send + 'static,
        U: Send,
        E: Send + 'static,
        F: FnOnce(T) -> BoxFuture<'static, Result<U, E>> + Send + 'static,
    {
        #[inline]
        fn flat_map_async(self, op: F) -> BoxFuture<'static, Result<U, E>> {
            use futures::FutureExt;
            async move {
                match self {
                    Ok(r) => op(r).await,
                    Err(e) => Err(e),
                }
            }
            .boxed()
        }
    }
    pub trait Exists<T, F: FnOnce(T) -> bool> {
        fn exists(self, f: F) -> bool;
    }
    impl<U, E, F: FnOnce(U) -> bool> Exists<U, F> for Result<U, E> {
        #[inline]
        fn exists(self, f: F) -> bool {
            match self {
                Ok(s) => f(s),
                Err(_) => false,
            }
        }
    }
}

pub mod option {
    use futures::future::BoxFuture;

    #[deprecated(note = "use `and_then` instead")]
    pub trait FlatMap<T, U, F: FnOnce(T) -> Option<U>> {
        fn flat_map(self, op: F) -> Option<U>;
    }
    #[allow(deprecated)]
    impl<T, U, F: FnOnce(T) -> Option<U>> FlatMap<T, U, F> for Option<T> {
        #[inline]
        fn flat_map(self, op: F) -> Option<U> {
            match self {
                Some(r) => op(r),
                None => None,
            }
        }
    }
    #[deprecated(note = "use `ok_or` instead")]
    pub trait ToResult<T, U, F: FnOnce() -> U> {
        fn to_result(self, err: F) -> Result<T, U>;
    }
    #[allow(deprecated)]
    impl<T, U, F: FnOnce() -> U> ToResult<T, U, F> for Option<T> {
        #[inline]
        fn to_result(self, err: F) -> Result<T, U> {
            match self {
                Some(s) => Ok(s),
                None => Err(err()),
            }
        }
    }
    pub trait ToVec<T> {
        fn to_vec(self) -> Vec<T>;
    }
    impl<T> ToVec<T> for Option<T> {
        #[inline]
        fn to_vec(self) -> Vec<T> {
            match self {
                Some(s) => vec![s],
                None => vec![],
            }
        }
    }
    #[deprecated(note = "use `is_some_and` instead")]
    pub trait Exists<T, F: FnOnce(T) -> bool> {
        fn exists(self, f: F) -> bool;
    }
    #[allow(deprecated)]
    impl<T, F: FnOnce(T) -> bool> Exists<T, F> for Option<T> {
        #[inline]
        fn exists(self, f: F) -> bool {
            match self {
                Some(s) => f(s),
                None => false,
            }
        }
    }

    #[deprecated(note = "use `is_none_or` instead")]
    pub trait ForAll<T, F: FnOnce(T) -> bool> {
        fn forall(self, f: F) -> bool;
    }
    #[allow(deprecated)]
    impl<T, F: FnOnce(T) -> bool> ForAll<T, F> for Option<T> {
        #[inline]
        fn forall(self, f: F) -> bool {
            match self {
                Some(s) => f(s),
                None => true,
            }
        }
    }

    pub trait AsyncFlatMap<T, U, E, F> {
        fn flat_map_async(self, op: F) -> BoxFuture<'static, Option<U>>;
    }

    impl<T, U, E, F> AsyncFlatMap<T, U, E, F> for Option<T>
    where
        T: Send + 'static,
        U: Send,
        E: Send + 'static,
        F: FnOnce(T) -> BoxFuture<'static, Option<U>> + Send + 'static,
    {
        #[inline]
        fn flat_map_async(self, op: F) -> BoxFuture<'static, Option<U>> {
            use futures::FutureExt;
            async move {
                match self {
                    Some(r) => op(r).await,
                    None => None,
                }
            }
            .boxed()
        }
    }
}

pub mod cow {
    use std::borrow::Cow;

    // to value by clone(bollowed) or not (owned)
    pub trait ToValue<T: Clone> {
        fn to_value(&self) -> &T;
    }
    impl<T: Clone> ToValue<T> for Cow<'_, T> {
        fn to_value(&self) -> &T {
            match self {
                Cow::Borrowed(b) => b.to_owned(),
                Cow::Owned(o) => o,
            }
        }
    }
}
pub mod string {
    pub trait ToOption<T> {
        fn to_option(self) -> Option<T>;
    }
    impl ToOption<String> for String {
        #[inline]
        fn to_option(self) -> Option<String> {
            if self.is_empty() {
                None
            } else {
                Some(self)
            }
        }
    }
}

// // https://stackoverflow.com/questions/65751826/how-can-i-lazy-initialize-fill-an-option-with-a-fallible-initializer
// trait TryGetOrInsert<T> {
//     fn get_or_insert_with<F>(&mut self, f: F) -> &mut T
//     where
//         F: FnOnce() -> T;
// }
// impl<T> TryGetOrInsert<T> for Option<T> {
//     fn _get_or_insert_with<F>(&mut self, f: F) -> &mut T
//     where
//         F: FnOnce() -> T,
//     {
//         match self {
//             Some(value) => value,
//             None => self.get_or_insert_with(f()),
//         }
//     }
// }

pub mod datetime {
    use anyhow::{anyhow, Context, Result};
    use chrono::{DateTime, Datelike, FixedOffset, LocalResult, TimeZone, Utc};
    use once_cell::sync::Lazy;
    use regex::Regex;

    pub static OFFSET_SEC: Lazy<i32> = Lazy::<i32>::new(|| {
        std::env::var("TZ_OFFSET_HOURS")
            .map_err(|e| -> anyhow::Error { e.into() })
            .and_then(|s| s.parse::<i32>().map_err(|e| -> anyhow::Error { e.into() }))
            .unwrap_or(9)
            * 3600
    });

    pub static TZ_OFFSET: Lazy<FixedOffset> =
        Lazy::<FixedOffset>::new(|| FixedOffset::east_opt(*OFFSET_SEC).unwrap());

    pub fn from_epoch_sec(epoch_sec: i64) -> DateTime<FixedOffset> {
        let utc_date_time = DateTime::from_timestamp_millis(epoch_sec * 1000).unwrap();
        utc_date_time.with_timezone(&*TZ_OFFSET)
    }

    pub fn from_epoch_milli(epoch_milli: i64) -> DateTime<FixedOffset> {
        DateTime::from_timestamp_millis(epoch_milli)
            .unwrap()
            .with_timezone(&*TZ_OFFSET)
    }

    pub fn now() -> DateTime<FixedOffset> {
        Utc::now().with_timezone(&FixedOffset::east_opt(*OFFSET_SEC).unwrap())
    }

    #[inline]
    pub fn now_millis() -> i64 {
        Utc::now().timestamp_millis()
    }

    pub fn now_nanos() -> i64 {
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    }

    pub fn now_seconds() -> i64 {
        now().timestamp()
    }

    pub fn ymdhms(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        min: u32,
        sec: u32,
    ) -> Option<DateTime<FixedOffset>> {
        match ymdhms_result(year, month, day, hour, min, sec) {
            Ok(dt) => Some(dt),
            Err(e) => {
                tracing::warn!(
                    "cannot create datetime: {}-{}-{} {}:{}:{}, {:?}",
                    year,
                    month,
                    day,
                    hour,
                    min,
                    sec,
                    e
                );
                None
            }
        }
    }

    pub fn ymdhms_result(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        min: u32,
        sec: u32,
    ) -> Result<DateTime<FixedOffset>> {
        match TZ_OFFSET.with_ymd_and_hms(year, month, day, hour, min, sec) {
            LocalResult::Single(res) => Ok(res),
            e => Err(anyhow!(
                "ymdhms error: {year}-{month}-{day} {hour}:{min}:{sec}, {e:?}"
            )),
        }
        //.ymd(year, month, day).and_hms(hour, min, sec)
    }
    // ymdhms regex  (sequencial)
    pub fn parse_datetime_ymdhms(
        dt_value: &str,
        datetime_regex: &Option<String>,
    ) -> Result<Option<DateTime<FixedOffset>>, anyhow::Error> {
        std::panic::catch_unwind(move || {
            let dt = match datetime_regex {
                Some(fmt) if !fmt.is_empty() => {
                    let now = self::now();
                    Regex::new(fmt.as_str())
                        .map(|dt_re| {
                            dt_re.captures(dt_value).and_then(|c| {
                                // XXX fixed sequence(ymdhms)
                                self::ymdhms(
                                    c.get(1).map_or(now.year(), |r| r.as_str().parse().unwrap()),
                                    c.get(2).map_or(0u32, |r| r.as_str().parse().unwrap()),
                                    c.get(3).map_or(0u32, |r| r.as_str().parse().unwrap()),
                                    c.get(4).map_or(0u32, |r| r.as_str().parse().unwrap()),
                                    c.get(5).map_or(0u32, |r| r.as_str().parse().unwrap()),
                                    c.get(6).map_or(0u32, |r| r.as_str().parse().unwrap()),
                                )
                            })
                        })
                        .context("on parse by datetime regex")
                }
                _ => DateTime::parse_from_rfc3339(dt_value)
                    .or_else(|_| DateTime::parse_from_str(dt_value, "%+"))
                    .map(Some)
                    .context("on parse rf3339"),
            };
            dt
        })
        .map_err(|e| {
            tracing::error!("caught panic: {:?}", e);
            anyhow!("error in parsing datetime: {e:?}")
        })
        .and_then(|dt| dt) // flatten
    }
}
pub mod text {
    use anyhow::{anyhow, Result};
    use regex::Regex;

    // https://stackoverflow.com/a/6041965
    const URL_REGEX: &str = r"((?:http|ftp|https):\/\/(:?[\w_-]+(?:(?:\.[\w_-]+)+))(?:[\w.,@?^=%&:\/~+#-]*[\w@?^=%&\/~+#-]))";
    pub fn extract_url_simple(message: &str) -> Option<&str> {
        let re = Regex::new(URL_REGEX).unwrap();
        re.captures(message)
            .and_then(|c| c.get(1).map(|s| s.as_str()))
    }
    /// Split text by specified delimiters or maximum length
    ///
    /// # Arguments
    /// * `text` - Text to split
    /// * `max_length` - Maximum length of each part (in bytes)
    /// * `delimiters` - Delimiter characters (in priority order)
    ///
    /// # Returns
    /// * `Result<Vec<String>>` - Split text strings
    pub fn split_text(text: &str, max_chars: usize, delimiters: &[&str]) -> Result<Vec<String>> {
        let mut parts = Vec::new();
        let mut char_start = 0;
        let char_count = text.chars().count();

        // Create character position to byte position mapping
        let char_byte_positions: Vec<usize> =
            text.char_indices().map(|(byte_pos, _)| byte_pos).collect();

        while char_start < char_count {
            let char_end = (char_start + max_chars).min(char_count);
            let byte_start = char_byte_positions[char_start];
            let byte_end = char_byte_positions
                .get(char_end)
                .copied()
                .unwrap_or(text.len());

            // Try splitting by delimiter characters
            let mut split_end = byte_end;
            if char_end < char_count {
                let substr = &text[byte_start..byte_end];
                for delimiter in delimiters {
                    if let Some(last_pos) = substr.rfind(delimiter) {
                        split_end = byte_start + last_pos + delimiter.len();
                        break;
                    }
                }
            }

            // Add valid substring
            if split_end > byte_start {
                parts.push(text[byte_start..split_end].to_string());
            } else {
                return Err(anyhow!("Invalid text splitting position"));
            }

            // Set next start position
            char_start = text[..split_end].chars().count();
        }

        Ok(parts)
    }

    // create test for extract_url_simple
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn test_extract_url_simple() {
            let url = "https://www.google.com/";
            let mes = format!("hello, <a href=\"{url}\">fuga</a>");
            assert_eq!(extract_url_simple(&mes), Some(url));
            let mes = format!("\"{url}\"");
            assert_eq!(extract_url_simple(&mes), Some(url));
            let mes = format!("<\"{url}\">");
            assert_eq!(extract_url_simple(&mes), Some(url));
            let mes = format!("\"<\"{url}\">\"");
            assert_eq!(extract_url_simple(&mes), Some(url));
        }
        #[test]
        fn test_extract_url_simple_with_queries() {
            let url = "https://www.google.com?q=hello&lang=en#top";
            let mes = format!("hello, {url}");
            assert_eq!(extract_url_simple(&mes), Some(url));
            let mes = format!("\"{url}\"");
            assert_eq!(extract_url_simple(&mes), Some(url));
            let mes = format!("<\"{url}\">");
            assert_eq!(extract_url_simple(&mes), Some(url));
            let mes = format!("\"<\"{url}\">\"");
            assert_eq!(extract_url_simple(&mes), Some(url));
        }

        #[test]
        fn test_split_japanese_text() -> Result<()> {
            let text = "吾輩は猫である。名前はまだ無い。どこで生れたかとんと見当がつかぬ。";
            let delimiters = &["。", "、"];
            let parts = split_text(text, 10, delimiters)?;

            assert_eq!(
                parts,
                vec![
                    "吾輩は猫である。",
                    "名前はまだ無い。",
                    "どこで生れたかとんと",
                    "見当がつかぬ。"
                ]
            );
            Ok(())
        }

        #[test]
        fn test_split_by_length() -> Result<()> {
            let text = "あいうiえお😁かきくjけこ🤨さしすkせそ.";
            let mut parts = split_text(text, 5, &[])?;

            assert_eq!(
                parts,
                vec!["あいうiえ", "お😁かきく", "jけこ🤨さ", "しすkせそ", "."]
            );
            // Remove the last element from parts if it's shorter than a certain length
            if let Some(last_part) = parts.last() {
                if last_part.chars().count() < 3 {
                    parts.pop();
                }
            }
            assert_eq!(
                parts,
                vec!["あいうiえ", "お😁かきく", "jけこ🤨さ", "しすkせそ"]
            );

            Ok(())
        }
    }
}

pub mod json {
    pub fn merge(a: &mut serde_json::Value, b: serde_json::Value) {
        if let serde_json::Value::Object(a) = a {
            if let serde_json::Value::Object(b) = b {
                for (k, v) in b {
                    if v.is_null() {
                        a.remove(&k);
                    } else {
                        merge(a.entry(k).or_insert(serde_json::Value::Null), v);
                    }
                }
                return;
            }
        }

        *a = b;
    }
    pub fn merge_obj(
        a: &mut serde_json::Map<String, serde_json::Value>,
        b: serde_json::Map<String, serde_json::Value>,
    ) {
        for (k, v) in b {
            if v.is_null() {
                a.remove(&k);
            } else {
                merge(a.entry(k).or_insert(serde_json::Value::Null), v);
            }
        }
    }
}
