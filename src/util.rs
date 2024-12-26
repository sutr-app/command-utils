pub mod encoding;
pub mod id_generator;
pub mod shutdown;
pub mod tracing;

pub mod result {
    use anyhow::Result;
    // use async_trait::async_trait;
    // use std::pin::Pin;

    use futures::future::BoxFuture;
    pub trait FlatMap<T, U, F: FnOnce(T) -> Result<U>> {
        fn flat_map(self, op: F) -> Result<U>;
    }
    impl<T, U, F: FnOnce(T) -> Result<U>> FlatMap<T, U, F> for Result<T> {
        #[inline]
        fn flat_map(self, op: F) -> Result<U> {
            match self {
                Ok(r) => op(r),
                Err(e) => Err(e),
            }
        }
    }
    pub trait ToOption<T, E> {
        fn to_option(self) -> Option<T>;
    }

    impl<T, E> ToOption<T, E> for Result<T, E> {
        #[inline]
        fn to_option(self) -> Option<T> {
            match self {
                Ok(r) => Some(r),
                Err(_e) => None,
            }
        }
    }

    pub trait Tap<T, E, F> {
        fn tap(self, f: F) -> Result<T, E>;
    }

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

    pub trait TapErr<T, E, F> {
        fn tap_err(self, f: F) -> Result<T, E>;
    }

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

    pub trait FlatMap<T, U, F: FnOnce(T) -> Option<U>> {
        fn flat_map(self, op: F) -> Option<U>;
    }
    impl<T, U, F: FnOnce(T) -> Option<U>> FlatMap<T, U, F> for Option<T> {
        #[inline]
        fn flat_map(self, op: F) -> Option<U> {
            match self {
                Some(r) => op(r),
                None => None,
            }
        }
    }
    pub trait ToResult<T, U, F: FnOnce() -> U> {
        fn to_result(self, err: F) -> Result<T, U>;
    }
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
    pub trait Exists<T, F: FnOnce(T) -> bool> {
        fn exists(self, f: F) -> bool;
    }
    impl<T, F: FnOnce(T) -> bool> Exists<T, F> for Option<T> {
        #[inline]
        fn exists(self, f: F) -> bool {
            match self {
                Some(s) => f(s),
                None => false,
            }
        }
    }

    pub trait ForAll<T, F: FnOnce(T) -> bool> {
        fn forall(self, f: F) -> bool;
    }
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
    use super::result::FlatMap;
    use anyhow::{anyhow, Result};
    use chrono::{DateTime, FixedOffset, LocalResult, TimeZone, Utc};
    use once_cell::sync::Lazy;

    pub static OFFSET_SEC: Lazy<i32> = Lazy::<i32>::new(|| {
        std::env::var("TZ_OFFSET_HOURS")
            .map_err(|e| e.into())
            .flat_map(|s| s.parse::<i32>().map_err(|e| e.into()))
            .unwrap_or(0)
            * 3600
    });

    pub static TZ_OFFSET: Lazy<FixedOffset> =
        Lazy::<FixedOffset>::new(|| FixedOffset::east_opt(*OFFSET_SEC).unwrap());

    pub fn from_epoch_sec(epoch_sec: i64) -> DateTime<FixedOffset> {
        let utc_date_time = DateTime::from_timestamp_millis(epoch_sec * 1000).unwrap();
        utc_date_time.with_timezone(&*TZ_OFFSET)
    }

    // XXX +9:00 +9:00 from epoch_milli
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
                "ymdhms error: {}-{}-{} {}:{}:{}, {:?}",
                year,
                month,
                day,
                hour,
                min,
                sec,
                e
            )),
        }
        //.ymd(year, month, day).and_hms(hour, min, sec)
    }
}
pub mod text {
    use super::option::FlatMap;
    use regex::Regex;

    // https://stackoverflow.com/a/6041965
    const URL_REGEX: &str = r"((?:http|ftp|https):\/\/(:?[\w_-]+(?:(?:\.[\w_-]+)+))(?:[\w.,@?^=%&:\/~+#-]*[\w@?^=%&\/~+#-]))";
    pub fn extract_url_simple(message: &str) -> Option<&str> {
        let re = Regex::new(URL_REGEX).unwrap();
        re.captures(message)
            .flat_map(|c| c.get(1).map(|s| s.as_str()))
    }

    // create test for extract_url_simple
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn test_extract_url_simple() {
            let url = "https://www.google.com/";
            let mes = format!("hello, <a href=\"{}\">fuga</a>", url);
            assert_eq!(extract_url_simple(&mes), Some(url));
            let mes = format!("\"{}\"", url);
            assert_eq!(extract_url_simple(&mes), Some(url));
            let mes = format!("<\"{}\">", url);
            assert_eq!(extract_url_simple(&mes), Some(url));
            let mes = format!("\"<\"{}\">\"", url);
            assert_eq!(extract_url_simple(&mes), Some(url));
        }
        #[test]
        fn test_extract_url_simple_with_queries() {
            let url = "https://www.google.com?q=hello&lang=en#top";
            let mes = format!("hello, {}", url);
            assert_eq!(extract_url_simple(&mes), Some(url));
            let mes = format!("\"{}\"", url);
            assert_eq!(extract_url_simple(&mes), Some(url));
            let mes = format!("<\"{}\">", url);
            assert_eq!(extract_url_simple(&mes), Some(url));
            let mes = format!("\"<\"{}\">\"", url);
            assert_eq!(extract_url_simple(&mes), Some(url));
        }
    }
}
