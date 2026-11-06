use std::error::Error;

use super::Identity;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ipnetwork::IpNetwork;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
extern crate self as viewset;
use crate::{Entity, Repository};

#[derive(Entity, FromRow, Serialize, Clone, Deserialize)]
#[entity(table = "sessions", create = "NewSession")]
pub struct Session {
    #[entity(skip_create)]
    pub id: Uuid,
    #[entity(sortable)]
    #[entity(skip_create)]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[entity(filterable)]
    pub sub: Uuid,
    pub username: String,
    pub email: String,
    pub expires_at: DateTime<Utc>,
    pub idle_until: DateTime<Utc>,
    pub ip_address: IpNetwork,
    #[entity(soft_delete)]
    pub deleted_at: chrono::DateTime<chrono::Utc>,
}

struct UpdateSession {
    idle_until: DateTime<Utc>,
}

use super::middleware::IdentityResult;

impl Session {
    pub(crate) fn identify(&self) -> IdentityResult {
        if self.is_valid() {
            IdentityResult::Expired
        } else {
            IdentityResult::Active(Identity {
                sub: self.sub,
                username: self.username.clone(),
                email: self.email.clone(),
            })
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
        let session = self.retrieve(&id).await?;
        match session.identify() {
            IdentityResult::Expired => Ok(None),
            IdentityResult::Active(id) => Ok(Some(id)),
        }
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
