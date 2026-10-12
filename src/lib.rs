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
extern crate self as viewset;
mod entity;
mod no_cache;
mod pagination;
mod repository;
#[cfg(feature = "web")]
pub mod web;
pub use entity::Entity;
pub use pagination::{Page, PaginationParams, SortDirection};
pub use repository::{DefaultRepo, Repository};
pub use viewset_macros::Entity;
#[cfg(feature = "web")]
pub use web::{ApiError, DefaultViewSet, Service, ViewSet};

use serde::Serialize;

pub type Filters = std::collections::HashMap<String, String>;

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

#[macro_export]
macro_rules! filters {
    // filters!(name, age)
    ( $( $k:ident ),* $(,)? ) => {
        &::std::collections::HashMap::<String, String>::from([
            $( (stringify!($k).to_string(), $k.to_string()) ),*
        ])
    };
    // filters!("name" => name)
    ( $( $k:expr => $v:expr ),* $(,)? ) => {
        &::std::collections::HashMap::<String, String>::from([
            $( (($k).to_string(), ($v).to_string()) ),*
        ])
    };
}
