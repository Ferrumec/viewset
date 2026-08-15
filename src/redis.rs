use redis::{AsyncCommands, aio::MultiplexedConnection};
use serde::{Serialize, de::DeserializeOwned};
use std::{marker::PhantomData, time::Duration};

pub struct RedisStore<T> {
    connection: MultiplexedConnection,
    prefix: String,
    _marker: PhantomData<fn() -> T>,
}

impl<T> RedisStore<T>
where
    T: Serialize + DeserializeOwned,
{
    pub async fn new(
        url: impl redis::IntoConnectionInfo,
        prefix: impl Into<String>,
    ) -> redis::RedisResult<Self> {
        let client = redis::Client::open(url)?;
        let connection = client.get_multiplexed_async_connection().await?;

        Ok(Self {
            connection,
            prefix: prefix.into(),
            _marker: PhantomData,
        })
    }

    fn key(&self, key: impl AsRef<str>) -> String {
        format!("{}:{}", self.prefix, key.as_ref())
    }

    pub async fn get(&mut self, key: impl AsRef<str>) -> redis::RedisResult<Option<T>> {
        let key = self.key(key);

        let value: Option<Vec<u8>> = self.connection.get(key).await?;

        value
            .map(|bytes| {
                serde_json::from_slice(&bytes).map_err(|err| {
                    redis::RedisError::from((
                        redis::ErrorKind::Parse,
                        "failed to deserialize Redis value",
                        err.to_string(),
                    ))
                })
            })
            .transpose()
    }

    pub async fn set(&mut self, key: impl AsRef<str>, value: &T) -> redis::RedisResult<()> {
        let key = self.key(key);

        let bytes = serde_json::to_vec(value).map_err(|err| {
            redis::RedisError::from((
                redis::ErrorKind::Parse,
                "failed to serialize value",
                err.to_string(),
            ))
        })?;

        self.connection.set(key, bytes).await
    }

    pub async fn set_ex(
        &mut self,
        key: impl AsRef<str>,
        value: &T,
        ttl: Duration,
    ) -> redis::RedisResult<()> {
        let key = self.key(key);

        let bytes = serde_json::to_vec(value).map_err(|err| {
            redis::RedisError::from((
                redis::ErrorKind::Parse,
                "failed to serialize value",
                err.to_string(),
            ))
        })?;

        self.connection.set_ex(key, bytes, ttl.as_secs()).await
    }

    pub async fn delete(&mut self, key: impl AsRef<str>) -> redis::RedisResult<bool> {
        let key = self.key(key);

        let deleted: usize = self.connection.del(key).await?;

        Ok(deleted > 0)
    }

    pub async fn exists(&mut self, key: impl AsRef<str>) -> redis::RedisResult<bool> {
        let key = self.key(key);

        self.connection.exists(key).await
    }
}
