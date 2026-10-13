use super::Identity;
use chrono::{DateTime, Utc};
use ipnetwork::IpNetwork;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
extern crate self as viewset;
use crate::Entity;

#[derive(Entity, FromRow, Serialize, Clone, Deserialize)]
#[entity(table = "sessions", create = "NewSession")]
pub struct Session {
    #[entity(skip_create)]
    pub id: Uuid,
    #[entity(sortable)]
    #[entity(skip_create)]
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub sub: Uuid,
    pub username: String,
    pub email: String,
    pub expires_at: DateTime<Utc>,
    pub ip_address: IpNetwork,
}

use super::middleware::IdentityResult;

impl Session {
    pub(crate) fn identify(&self) -> IdentityResult {
        if chrono::Utc::now() > self.expires_at {
            IdentityResult::Expired
        } else {
            IdentityResult::Active(Identity {
                sub: self.sub,
                username: self.username.clone(),
                email: self.email.clone(),
            })
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct NewSession {
    pub sub: Uuid,
    pub username: String,
    pub email: String,
    pub expires_at: DateTime<Utc>,
    pub ip_address: IpNetwork,
}
use crate::DefaultRepo;
pub type SessionRepo = DefaultRepo<Session>;
