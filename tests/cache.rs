//! Behavioral tests for the `#[cache]` attribute macro.
//!
//! Regression coverage for:
//! - cache key must incorporate the actual argument values (not a constant)
//! - `ttl = N` must be threaded into the stored TTL
//! - `key = "..."` must be used as the key template
//! - `tag = "..."` must route through the tagged cache with the tags attached

use armature_proc_macro::cache;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

/// A minimal in-memory cache exposing the surface the generated code calls.
#[derive(Default)]
struct MockCache {
    store: Mutex<HashMap<String, String>>,
    sets: Mutex<Vec<(String, Option<Duration>)>>,
}

impl MockCache {
    fn new() -> Self {
        Self::default()
    }

    async fn get_json(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.store.lock().unwrap().get(key).cloned())
    }

    async fn set_json(
        &self,
        key: &str,
        value: String,
        ttl: Option<Duration>,
    ) -> Result<(), String> {
        self.store.lock().unwrap().insert(key.to_string(), value);
        self.sets.lock().unwrap().push((key.to_string(), ttl));
        Ok(())
    }
}

mod key_from_args {
    use super::*;

    #[allow(non_upper_case_globals)]
    static __cache: LazyLock<MockCache> = LazyLock::new(MockCache::new);

    #[cache]
    async fn get_user(id: i64) -> Result<String, String> {
        Ok(format!("user-{id}"))
    }

    #[tokio::test]
    async fn distinct_args_do_not_collide() {
        let a = get_user(1).await.unwrap();
        let b = get_user(2).await.unwrap();
        // The bug: one constant key for all args means get_user(2) returns the
        // cached value of get_user(1).
        assert_eq!(a, "user-1");
        assert_eq!(b, "user-2");

        // Two distinct keys must have been written.
        let keys: Vec<String> = {
            let sets = __cache.sets.lock().unwrap();
            sets.iter().map(|(k, _)| k.clone()).collect()
        };
        assert_eq!(keys.len(), 2, "expected two cache writes");
        assert_ne!(keys[0], keys[1], "distinct args must produce distinct keys");

        // A repeat call for id=1 must hit the cache (no third write).
        let a2 = get_user(1).await.unwrap();
        assert_eq!(a2, "user-1");
        assert_eq!(
            __cache.sets.lock().unwrap().len(),
            2,
            "cached hit must not write again"
        );
    }
}

mod ttl_threaded {
    use super::*;

    #[allow(non_upper_case_globals)]
    static __cache: LazyLock<MockCache> = LazyLock::new(MockCache::new);

    #[cache(ttl = 5)]
    async fn get_posts(user_id: i64) -> Result<String, String> {
        Ok(format!("posts-{user_id}"))
    }

    #[tokio::test]
    async fn ttl_attribute_is_used() {
        let _ = get_posts(42).await.unwrap();
        let sets = __cache.sets.lock().unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].1, Some(Duration::from_secs(5)));
    }
}

mod custom_key {
    use super::*;

    #[allow(non_upper_case_globals)]
    static __cache: LazyLock<MockCache> = LazyLock::new(MockCache::new);

    #[cache(key = "user:profile:{}", ttl = 600)]
    async fn get_profile(user_id: i64) -> Result<String, String> {
        Ok(format!("profile-{user_id}"))
    }

    #[tokio::test]
    async fn key_template_is_applied() {
        let _ = get_profile(7).await.unwrap();
        let sets = __cache.sets.lock().unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].0, "user:profile:7");
        assert_eq!(sets[0].1, Some(Duration::from_secs(600)));
    }
}

// ---------------------------------------------------------------------------
// Key-collision regressions (M13).
// ---------------------------------------------------------------------------

/// One cache shared by several functions, so cross-function collisions show.
static SHARED: LazyLock<MockCache> = LazyLock::new(MockCache::new);

mod template_delimiter_injection {
    use super::SHARED as __cache;
    use super::*;

    #[cache(key = "pair:{}:{}")]
    async fn pair(a: String, b: String) -> Result<String, String> {
        Ok(format!("{a}|{b}"))
    }

    #[tokio::test]
    async fn interpolated_values_cannot_forge_delimiters() {
        // Under plain `format!` both calls rendered `pair:1:2:3` and the second
        // call was served the first call's result.
        let first = pair("1:2".into(), "3".into()).await.unwrap();
        let second = pair("1".into(), "2:3".into()).await.unwrap();
        assert_eq!(first, "1:2|3");
        assert_eq!(second, "1|2:3");
        let keys: Vec<String> = __cache
            .sets
            .lock()
            .unwrap()
            .iter()
            .map(|(k, _)| k.clone())
            .collect();
        assert!(keys.contains(&"pair:1%3A2:3".to_string()), "{keys:?}");
        assert!(keys.contains(&"pair:1:2%3A3".to_string()), "{keys:?}");
    }
}

mod destructured_and_scoped {
    use super::*;

    #[allow(non_upper_case_globals)]
    static __cache: LazyLock<MockCache> = LazyLock::new(MockCache::new);

    pub struct Path<T>(pub T);

    // Destructured arguments used to be dropped from the key entirely, so
    // every call shared a single entry.
    #[cache]
    async fn by_path(Path(id): Path<u64>, (page, size): (u32, u32)) -> Result<String, String> {
        Ok(format!("{id}/{page}/{size}"))
    }

    #[tokio::test]
    async fn destructured_arguments_participate_in_the_key() {
        assert_eq!(by_path(Path(1), (1, 10)).await.unwrap(), "1/1/10");
        assert_eq!(by_path(Path(2), (1, 10)).await.unwrap(), "2/1/10");
        assert_eq!(by_path(Path(1), (2, 10)).await.unwrap(), "1/2/10");
        assert_eq!(by_path(Path(1), (1, 20)).await.unwrap(), "1/1/20");
        // The cache is shared with the test below, so count only this fn's keys.
        let writes = __cache
            .sets
            .lock()
            .unwrap()
            .iter()
            .filter(|(k, _)| k.contains("by_path"))
            .count();
        assert_eq!(writes, 4);
    }

    // Default keys of equally named functions must not collide, even when
    // they share one cache: the key names the module, `Self` type, location
    // and return type.
    pub struct Users;
    pub struct Orders;

    impl Users {
        #[cache]
        pub async fn get(&self, id: u64) -> Result<String, String> {
            Ok(format!("user-{id}"))
        }
    }

    impl Orders {
        #[cache]
        pub async fn get(&self, id: u64) -> Result<String, String> {
            Ok(format!("order-{id}"))
        }

        #[cache]
        pub async fn find(id: u64) -> Result<String, String> {
            Ok(format!("order-find-{id}"))
        }
    }

    impl Users {
        #[cache]
        pub async fn find(id: u64) -> Result<String, String> {
            Ok(format!("user-find-{id}"))
        }
    }

    #[tokio::test]
    async fn same_named_methods_on_different_types_do_not_collide() {
        assert_eq!(Users.get(1).await.unwrap(), "user-1");
        assert_eq!(Orders.get(1).await.unwrap(), "order-1");
        assert_eq!(Orders::find(1).await.unwrap(), "order-find-1");
        assert_eq!(Users::find(1).await.unwrap(), "user-find-1");
        let sets = __cache.sets.lock().unwrap();
        let user_key = sets
            .iter()
            .map(|(k, _)| k)
            .find(|k| k.contains("Users::get"))
            .expect("Users::get key");
        assert!(user_key.contains(module_path!()), "{user_key}");
    }
}

mod string_args_are_length_prefixed {
    use super::*;

    #[allow(non_upper_case_globals)]
    static __cache: LazyLock<MockCache> = LazyLock::new(MockCache::new);

    #[cache]
    async fn join(a: String, b: String) -> Result<String, String> {
        Ok(format!("{a}+{b}"))
    }

    #[tokio::test]
    async fn argument_boundaries_cannot_shift() {
        assert_eq!(join("a,".into(), "b".into()).await.unwrap(), "a,+b");
        assert_eq!(join("a".into(), ",b".into()).await.unwrap(), "a+,b");
        assert_eq!(join("ab".into(), "".into()).await.unwrap(), "ab+");
        assert_eq!(join("a".into(), "b".into()).await.unwrap(), "a+b");
        assert_eq!(__cache.sets.lock().unwrap().len(), 4);
    }
}

mod unencodable_args_bypass_the_cache {
    use super::*;
    use std::collections::BTreeMap;

    #[allow(non_upper_case_globals)]
    static __cache: LazyLock<MockCache> = LazyLock::new(MockCache::new);

    // JSON cannot encode a map with non-string keys: the call must run
    // uncached rather than fall back to a colliding key.
    #[cache]
    async fn lookup(map: BTreeMap<(u8, u8), u8>) -> Result<usize, String> {
        Ok(map.len())
    }

    #[tokio::test]
    async fn encoding_failure_runs_the_function_uncached() {
        let mut m = BTreeMap::new();
        m.insert((1, 2), 3);
        assert_eq!(lookup(m).await.unwrap(), 1);
        assert!(__cache.sets.lock().unwrap().is_empty());
    }
}
