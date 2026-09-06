//! actixutils viewset
//!
//! A generic, Django-REST-Framework-inspired CRUD toolkit for building
//! admin REST APIs on top of actix-web + sqlx + Postgres.
//!
//! Layering (request flows top to bottom, response flows bottom to top):
//!
//!   HTTP request -> ViewSet -> Service -> Repository -> Database
//!
//! Each layer is a trait with default (mostly no-op or delegating)
//! implementations, so a new entity only needs a handful of `impl` blocks
//! plus entity metadata to get a fully working CRUD API.

mod context;
mod entity;
mod error;
mod pagination;
mod repository;
mod service;
mod sql;
#[allow(clippy::module_inception)]
mod viewset;
pub use context::RequestContext;
pub use entity::Entity;
pub use error::ApiError;
pub use pagination::{Page, PaginationParams, SortDirection};
pub use repository::{DefaultRepo, Repository};
pub use service::{DefaultService, Service};
pub use sql::{Field, SqlType, SqlValue};
pub use viewset::{DefaultViewSet, ViewSet};
pub use viewset_macros::Entity;

use serde::Serialize;

/// Default `UpdateDto` for entities that never specify one. This type has
/// no values — it can't be constructed — so any code path that would need
/// one is provably unreachable. `web::Json<NoUpdateDto>` extraction fails
/// deserialization for *every* request body, so PUT/PATCH on such a
/// resource is rejected before it ever reaches `Service::update`.
#[derive(Clone, Serialize)]
pub enum NoUpdateDto {}

impl<'de> serde::Deserialize<'de> for NoUpdateDto {
    fn deserialize<D>(_d: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Err(serde::de::Error::custom(
            "this resource does not support update",
        ))
    }
}
