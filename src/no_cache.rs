use ferrumec::Store;
use std::error::Error;
use std::marker::PhantomData;
use std::sync::Arc;

pub struct NoCache<K, V> {
    _marker: PhantomData<(K, V)>,
}

#[async_trait::async_trait]
impl<K, V> Store<K, V> for NoCache<K, V>
where
    K: Send + Sync + 'static,
    V: Send + Sync + 'static,
{
    async fn get(&self, _key: &K) -> Result<Option<V>, Box<dyn Error>> {
        Ok(None)
    }

    async fn set(&self, _key: &K, _value: V) -> Result<(), Box<dyn Error>> {
        Ok(())
    }

    async fn delete(&self, _key: &K) -> Result<(), Box<dyn Error>> {
        Ok(())
    }
}

impl<K, V> NoCache<K, V> {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            _marker: PhantomData,
        })
    }
}
