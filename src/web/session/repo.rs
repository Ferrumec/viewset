use super::Identity;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use ipnetwork::IpNetwork;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use sqlx::PgPool;
use std::error::Error;
use std::sync::Arc;
use uuid::Uuid;
extern crate self as viewset;
use crate::{Entity, Repository};

#[derive(Entity, FromRow, Serialize, Clone, Deserialize)]
#[entity(table = "sessions", create = "NewSession", update = "UpdateSession")]
pub struct Session {
    #[entity(skip_create, skip_update)]
    pub id: Uuid,
    #[entity(sortable)]
    #[entity(skip_create, skip_update)]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[entity(filterable, skip_update)]
    pub sub: Uuid,
    #[entity(skip_update)]
    pub username: String,
    #[entity(skip_update)]
    pub email: String,
    #[entity(skip_update)]
    pub expires_at: DateTime<Utc>,
    pub idle_until: DateTime<Utc>,
    #[entity(skip_update)]
    pub ip_address: IpNetwork,
    #[entity(soft_delete)]
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Serialize, Clone, Deserialize)]
pub struct UpdateSession {
    idle_until: Option<DateTime<Utc>>,
}

use super::middleware::IdentityResult;

impl Session {
    pub(crate) fn identify(&self) -> IdentityResult {
        if self.is_valid() {
            IdentityResult::Active(Identity {
                sub: self.sub,
                username: self.username.clone(),
                email: self.email.clone(),
            })
        } else {
            IdentityResult::Expired
        }
    }

    fn is_valid(&self) -> bool {
        self.expires_at > Utc::now() && self.idle_until > Utc::now()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct NewSession {
    pub sub: Uuid,
    pub username: String,
    pub email: String,
    pub expires_at: DateTime<Utc>,
    pub ip_address: IpNetwork,
    pub idle_until: DateTime<Utc>,
}

use std::collections::HashMap;
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct SessionRepo {
    db: PgPool,
    cache: Arc<dyn Store<Uuid, Session>>,
    list_cache: Arc<dyn Store<u64, (Vec<Session>, i64)>>,
    list_one_cache: Arc<dyn Store<u64, Session>>,
    session_idle_duration: Duration,
    tx: mpsc::Sender<(Uuid, DateTime<Utc>)>,
}

impl SessionRepo {
    pub fn new(db: PgPool, cache_factory: impl CacheFactory) -> SessionRepo {
        let (tx, mut rx) = mpsc::channel(100);
        let value = db.clone();
        tokio::spawn(async move {
            let mut map: HashMap<Uuid, DateTime<Utc>> = HashMap::new();

            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(300));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    Some((key, val)) = rx.recv() => {
                        map.insert(key, val);
                    }
                    _ = ticker.tick() => {
                        for (key, val) in map.drain() {
                            if let Err(e) = sqlx::query("update sessions set idle_until = $1 where id = $2")
                                .bind(val)
                                .bind(key)
                                .execute(&value)
                                .await
                            {
                                tracing::warn!("failed to persist session idle_until: {e}");
                            }
                        }
                    }
                    else => break,
                }
            }
        });
        Self {
            db,
            cache: cache_factory.new_cache(
                &format!("{}_items", Session::TABLE),
                std::time::Duration::from_mins(30),
            ),
            list_cache: cache_factory.new_cache(
                &format!("{}_lists", Session::TABLE),
                std::time::Duration::from_mins(30),
            ),
            list_one_cache: cache_factory.new_cache(
                &format!("{}_list_one", Session::TABLE),
                std::time::Duration::from_mins(30),
            ),
            session_idle_duration: Duration::days(
                std::env::var("SESSION_IDLE_DAYS")
                    .unwrap_or("1".to_string())
                    .parse::<i64>()
                    .unwrap_or(1),
            ),
            tx,
        }
    }
}

impl Repository for SessionRepo {
    type Entity = Session;
    fn cache(&self) -> Arc<dyn Store<Uuid, Session> + Send + Sync> {
        self.cache.clone()
    }
    fn list_cache(&self) -> Arc<dyn Store<u64, (Vec<Session>, i64)> + Send + Sync> {
        self.list_cache.clone()
    }
    fn list_one_cache(&self) -> Arc<dyn Store<u64, Session> + Send + Sync> {
        self.list_one_cache.clone()
    }
    fn database(&self) -> &PgPool {
        &self.db
    }
}

use ferrumec::{CacheFactory, Store};

#[async_trait]
impl Store<Uuid, Identity> for SessionRepo {
    async fn get(&self, id: &Uuid) -> Result<Option<Identity>, Box<dyn Error>> {
        // An unknown or soft-deleted session is "not logged in" (401), not a
        // store failure (500).
        let session = match self.retrieve(&id).await {
            Ok(s) => s,
            Err(sqlx::Error::RowNotFound) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let ident = match session.identify() {
            IdentityResult::Expired => return Ok(None),
            IdentityResult::Active(id) => id,
        };
        if let Err(e) = self
            .tx
            .send((session.id, Utc::now() + self.session_idle_duration))
            .await
        {
            tracing::warn!("failed to send session idle timeout update: {e}")
        };
        Ok(Some(ident))
    }

    async fn delete(&self, id: &Uuid) -> Result<(), Box<dyn Error>> {
        Repository::delete(self, &id).await?;
        Ok(())
    }
    async fn set(&self, _k: &Uuid, _v: Identity) -> Result<(), Box<dyn Error>> {
        Ok(())
    }

    async fn clear(&self) -> Result<(), Box<dyn Error>> {
        Ok(())
    }
}
