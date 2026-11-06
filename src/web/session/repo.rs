use std::error::Error;

use super::Identity;
use async_trait::async_trait;
use chrono::{DateTime, Utc, Duration};
use ipnetwork::IpNetwork;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
extern crate self as viewset;
use crate::{Entity, Repository};

#[derive(Entity, FromRow, Serialize, Clone, Deserialize)]
#[entity(table = "sessions", create = "NewSession", update = "UpdateSession")]
pub struct Session {
    #[entity(skip_create,skip_update)]
    pub id: Uuid,
    #[entity(sortable)]
    #[entity(skip_create,skip_update)]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[entity(filterable,skip_update)]
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
use crate::DefaultRepo;
pub type SessionRepo = DefaultRepo<Session>;

use ferrumec::Store;

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
        self.update(id,UpdateSession{idle_until: Some(Utc::now()+Duration::days(1))}).await?;
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
