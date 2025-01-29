use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use tokio::sync::RwLock;
/// Scoped cache that can be used in async context
pub struct ScopedCache<K, V>
where
    K: Eq + Hash,
{
    cache: Arc<RwLock<HashMap<K, V>>>,
}

impl<K, V> Default for ScopedCache<K, V>
where
    K: Eq + Hash,
{
    fn default() -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl<K, V> ScopedCache<K, V>
where
    K: Eq + Hash,
{
    /// Create a new empty cache
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a value in the cache
    pub async fn store(&self, key: K, value: V) {
        let mut cache = self.cache.write().await;
        cache.insert(key, value);
    }

    /// Retrieve a value from the cache
    pub async fn get(&self, key: &K) -> Option<V>
    where
        V: Clone,
    {
        let cache = self.cache.read().await;
        cache.get(key).cloned()
    }
}

/// Creates a new scoped cache context
pub async fn with_cache<F, Fut, K, V>(f: F) -> Fut::Output
where
    F: FnOnce(ScopedCache<K, V>) -> Fut,
    Fut: std::future::Future,
    K: Eq + Hash,
{
    let cache = ScopedCache::new();
    f(cache).await
}

// `cache(key) { heavy_computation() }` という形式で使えるようにするためのマクロ
#[macro_export]
macro_rules! cache {
    ($cache:expr, $key:expr, $heavy_computation:expr) => {
        if let Some(value) = $cache.get(&$key).await {
            value
        } else {
            let value = $heavy_computation.await;
            $cache.store($key, value.clone()).await;
            value
        }
    };
}
// `cache_ok(key) { heavy_computation_result() }` という形式でheavy_computation_result()の結果がOKの場合のみキャッシュするような形で使えるようにするためのマクロ
#[macro_export]
macro_rules! cache_ok {
    ($cache:expr, $key:expr, $heavy_computation_result:expr) => {
        if let Some(value) = $cache.get(&$key).await {
            Ok(value)
        } else {
            let value = $heavy_computation_result.await;
            if let Ok(v) = &value {
                $cache.store($key, v.clone()).await;
            }
            value
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_scoped_cache() {
        with_cache(|cache| async move {
            cache.store("key1", "value1").await;
            cache.store("key2", "value2").await;

            assert_eq!(cache.get(&"key1").await, Some("value1"));
            assert_eq!(cache.get(&"key2").await, Some("value2"));
            assert_eq!(cache.get(&"key3").await, None);
        })
        .await;
    }
    #[tokio::test]
    async fn test_scoped_cache_isolation() {
        // First scope
        let value1 = with_cache(|cache| async move {
            cache.store("key1", "value1").await;
            cache.get(&"key1").await
        })
        .await;
        assert_eq!(value1, Some("value1"));

        // Second scope with same key
        let value2 = with_cache(|cache| async move {
            // Should be None because this is a new cache
            assert_eq!(cache.get(&"key1").await, None);

            // Store a different value
            cache.store("key1", "value2").await;
            cache.get(&"key1").await
        })
        .await;
        assert_eq!(value2, Some("value2"));
    }
    #[tokio::test]
    async fn test_scoped_cache_macro() {
        let value = with_cache(|cache| async move {
            let key = "key";
            cache!(cache, key, async {
                sleep(Duration::from_secs(1)).await; // Heavy computation
                "value"
            })
        })
        .await;
        assert_eq!(value, "value");
    }

    #[tokio::test]
    async fn test_scoped_cache_macro_isolation() {
        async fn heavy_computation() -> Result<&'static str> {
            // sleep(Duration::from_secs(10)).await; // Heavy computation
            Ok("value1")
        }
        // First scope
        let value1: Result<&str> = with_cache(|cache| async move {
            let key = "key";
            assert!(cache_ok!(cache, key, heavy_computation()).unwrap() == "value1");
            cache_ok!(cache, key, heavy_computation())
        })
        .await;
        assert!(value1.is_ok());
        assert_eq!(value1.unwrap(), "value1");

        // Second scope with same key
        let value2 = with_cache(|cache| async move {
            let key = "key";
            cache!(cache, key, async {
                // sleep(Duration::from_secs(10)).await; // Heavy computation
                "value2"
            })
        })
        .await;
        assert_eq!(value2, "value2");
    }
    #[tokio::test]
    async fn test_scoped_cache_macro_nested() {
        let value = with_cache(|cache| async move {
            let key1 = "key1";
            let key2 = "key2";
            cache!(cache, key1, async {
                cache!(cache, key2, async {
                    sleep(Duration::from_secs(1)).await; // Heavy computation
                    "value"
                })
            })
        })
        .await;
        assert_eq!(value, "value");
    }
    #[tokio::test]
    async fn test_scoped_cache_nested() {
        async fn heavy_computation_once(value: &'static str) -> Result<&'static str> {
            // 2回以上同じvalueで呼ばれると失敗(panic)する
            static mut COUNT: i32 = 0;
            unsafe {
                COUNT += 1;
                if COUNT > 1 {
                    panic!("called more than once");
                }
            }
            Ok(value)
        }
        async fn heavy_computation(value: &'static str) -> Result<&'static str> {
            Ok(value)
        }
        let value = with_cache(|cache| async move {
            let key1 = "key1";
            let v1 = cache_ok!(cache, key1, heavy_computation("value1")).unwrap();
            assert!(v1 == "value1");
            let v2 = with_cache(|cache| async move {
                // let key1 = "key1";
                cache_ok!(cache, key1, heavy_computation_once("value2"));
                cache_ok!(cache, key1, heavy_computation_once("value2"))
            })
            .await;
            let v1 = cache_ok!(cache, key1, heavy_computation("value3")).unwrap();
            // from cache
            assert!(v1 == "value1");
            v2
        })
        .await;
        assert!(value.is_ok());
    }
}
