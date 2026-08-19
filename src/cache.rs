use actixutils::Store as Cache;
use moka::sync::Cache as MokaCache;
use std::error::Error;

pub struct DefaultCache<K, V> {
    cache: MokaCache<K, V>,
}

impl<K, V> DefaultCache<K, V>
where
    K: Eq + std::hash::Hash + Clone + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    pub fn new(max_capacity: u64) -> Self {
        Self {
            cache: MokaCache::builder().max_capacity(max_capacity).build(),
        }
    }
}

#[async_trait::async_trait]
impl<K, V> Cache<K, V> for DefaultCache<K, V>
where
    K: Eq + std::hash::Hash + Clone + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    async fn get(&self, key: &K) -> Result<Option<V>, Box<dyn Error>> {
        Ok(self.cache.get(key))
    }

    async fn set(&self, key: &K, value: V) -> Result<(), Box<dyn Error>> {
        self.cache.insert(key.clone(), value);
        Ok(())
    }

    async fn delete(&self, key: &K) -> Result<(), Box<dyn Error>> {
        self.cache.invalidate(key);
        Ok(())
    }

    async fn clear(&self) -> Result<(), Box<dyn Error>> {
        self.cache.invalidate_all();
        Ok(())
    }
}

impl<K, V> From<MokaCache<K, V>> for DefaultCache<K, V> {
    fn from(cache: MokaCache<K, V>) -> Self {
        Self { cache }
    }
}
