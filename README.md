# viewset

[![Crates.io](https://img.shields.io/crates/v/viewset.svg)](https://crates.io/crates/viewset)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)

A Rust, Actix Web–based resource framework that provides convention-driven CRUD APIs over database entities.

Inspired by Django REST Framework’s ViewSets, `viewset` gives you a layered **ViewSet → Service → Repository** architecture with validation/hooks, pagination, filtering, sorting, search, soft deletes, transactions, and optional caching — all driven by entity metadata and sensible defaults.

```
HTTP request → ViewSet → Service → Repository → Database
```

Each layer is a trait with default (mostly no-op or delegating) implementations, so a new entity only needs a handful of `impl` blocks plus entity metadata to get a fully working CRUD API.

## Features

- **Convention-over-configuration CRUD** – list, retrieve, create, update (PUT/PATCH), delete out of the box
- **Derive-powered entities** – `#[derive(Entity)]` generates table/column metadata, insert/update helpers, and DTOs
- **Pagination** – `?page=` / `?page_size=` with sensible defaults and hard limits
- **Filtering, sorting & search** – `?field=value`, `?sort=field,-other`, `?search=term`
- **Soft deletes** – optional boolean or timestamp column support
- **Transactional hooks** – `before_*` / `after_*` hooks for create/update/delete run inside a single transaction
- **Caching** – pluggable cache-aside / write-through via `ferrumec` (disabled by default)
- **Session middleware** (optional) – cookie-based sessions with identity extraction
- **Feature-gated web layer** – core repository/entity logic works without Actix; enable `web` for HTTP

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
viewset = { version = "0.6", features = ["web"] }
# Required companions
sqlx = { version = "0.9", features = ["runtime-tokio", "postgres", "uuid", "chrono", "rust_decimal"] }
actix-web = "4"
serde = { version = "1", features = ["derive"] }
uuid = { version = "1", features = ["serde", "v4"] }
```

The `web` feature pulls in `actix-web`, `chrono`, `uuid`, and `futures-util`.

## Quick Start

### 1. Define an entity

```rust
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use viewset::Entity;

#[derive(Debug, Clone, Serialize, FromRow, Entity)]
#[entity(table = "products")]
pub struct Product {
    pub id: Uuid,
    pub name: String,
    pub price: rust_decimal::Decimal,
    pub description: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

// The derive generates:
// - Entity::TABLE, COLUMNS, PK_COLUMN, etc.
// - CreateDto / UpdateDto (or you can supply your own)
// - insert() / update() helpers
// - id() extractor
```

You typically also provide (or let the macro generate) the associated types:

```rust
#[derive(Deserialize)]
pub struct CreateProduct {
    pub name: String,
    pub price: rust_decimal::Decimal,
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateProduct {
    pub name: Option<String>,
    pub price: Option<rust_decimal::Decimal>,
    pub description: Option<String>,
}

// ResponseDto can simply be the entity itself
impl From<Product> for Product {
    fn from(p: Product) -> Self { p }
}
```

> See the `viewset-macros` documentation and the `examples/` directory (when present) for the full set of attributes (`searchable`, `sortable`, `filterable`, soft-delete columns, custom DTOs, etc.).

### 2. Implement Repository / Service / ViewSet

Most of the time the defaults are enough:

```rust
use std::sync::Arc;
use sqlx::PgPool;
use viewset::{DefaultRepo, DefaultService, DefaultViewSet, Repository, Service, ViewSet};

// Repository
pub struct ProductRepo {
    pool: PgPool,
}

impl Repository for ProductRepo {
    type Entity = Product;
    fn database(&self) -> &PgPool {
        &self.pool
    }
}

// Or use the provided default:
let repo = DefaultRepo::<Product>::new(pool);

// Service (adds transactional hooks)
let service = DefaultService::from(repo);

// ViewSet (HTTP layer)
let viewset = Arc::new(DefaultViewSet::from(service));
```

### 3. Mount the routes

```rust
use actix_web::{web, App, HttpServer};

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let pool = /* create PgPool */;
    let repo = ProductRepo { pool };
    let service = DefaultService::from(repo);
    let vs = Arc::new(DefaultViewSet::from(service));

    HttpServer::new(move || {
        App::new().configure(|cfg| {
            vs.clone().configure(cfg, "/api/products");
        })
    })
    .bind(("0.0.0.0", 8080))?
    .run()
    .await
}
```

This registers:

| Method   | Path                  | Action    |
|----------|-----------------------|-----------|
| `GET`    | `/api/products`       | List      |
| `POST`   | `/api/products`       | Create    |
| `GET`    | `/api/products/{id}`  | Retrieve  |
| `PUT`    | `/api/products/{id}`  | Update    |
| `PATCH`  | `/api/products/{id}`  | Update    |
| `DELETE` | `/api/products/{id}`  | Delete    |

## Query Parameters

### Pagination

| Param        | Default | Description                          |
|--------------|---------|--------------------------------------|
| `page`       | `1`     | 1-based page number                  |
| `page_size`  | `25`    | Items per page (clamped to 1–200)    |

Response shape:

```json
{
  "items": [ ... ],
  "page": 1,
  "page_size": 25,
  "total": 142,
  "total_pages": 6
}
```

### Sorting

```
?sort=name          # ascending
?sort=-created_at   # descending
?sort=name,-price   # multiple fields
```

Only columns listed in `Entity::SORTABLE` are accepted.

### Filtering

```
?status=active&category=electronics
```

Only columns listed in `Entity::FILTERABLE` are accepted (exact match).

### Full-text-ish search

```
?search=wireless
```

Performs `ILIKE '%term%'` against columns listed in `Entity::SEARCHABLE`.

## Customizing Behaviour

### Service hooks

Override any of the lifecycle hooks to add validation, authorization, side-effects, etc. All mutation hooks run inside a single database transaction:

```rust
#[async_trait]
impl Service for MyProductService {
    type Repository = ProductRepo;

    fn repository(&self) -> &Self::Repository { &self.repo }

    async fn before_create(
        &self,
        _tx: &mut Transaction<'_, Postgres>,
        dto: CreateProduct,
    ) -> ApiResult<CreateProduct> {
        if dto.price.is_sign_negative() {
            return Err(ApiError::Validation("price must be non-negative".into()));
        }
        Ok(dto)
    }

    async fn after_create(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        entity: Product,
    ) -> ApiResult<Product> {
        // e.g. write an audit log in the same transaction
        Ok(entity)
    }

    // before_update / after_update / before_delete / after_delete
    // before_list / after_list
}
```

### Repository overrides

The default `Repository` methods build SQL dynamically from `Entity` metadata. Override any method when you need joins, window functions, custom filters, etc.:

```rust
#[async_trait]
impl Repository for ProductRepo {
    type Entity = Product;

    fn database(&self) -> &PgPool { &self.pool }

    async fn list(&self, query: &Filters) -> Result<(Vec<ProductResponse>, i64), sqlx::Error> {
        // custom query with joins...
    }
}
```

### Caching

By default repositories use a no-op cache. To enable caching, override the cache accessors:

```rust
fn cache(&self) -> Arc<dyn Store<Product::Id, Product> + Send + Sync> {
    // return a ferrumec-backed cache
}
```

List and single-item list caches are also available (`list_cache`, `list_one_cache`). Mutations automatically invalidate the relevant caches.

### Soft deletes

Declare a soft-delete column on the entity:

```rust
#[entity(table = "products", soft_delete = "deleted_at")]
// or soft_delete = "is_deleted" for a boolean flag
```

`delete` will then perform an update instead of a hard `DELETE`, and list/retrieve queries automatically exclude soft-deleted rows.

### Disabling updates

If a resource should be immutable after creation, use the provided `NoUpdateDto`:

```rust
type UpdateDto = viewset::NoUpdateDto;
```

Any PUT/PATCH request will fail deserialization before reaching the service layer.

## Error Handling

All layers converge on `ApiError` (under the `web` feature):

| Variant          | HTTP Status | Notes                                      |
|------------------|-------------|--------------------------------------------|
| `NotFound`       | 404         | Also mapped from `sqlx::Error::RowNotFound`|
| `Validation`     | 422         |                                            |
| `Forbidden`      | 403         |                                            |
| `Unauthorized`   | 401         |                                            |
| `Conflict`       | 409         |                                            |
| `StaleVersion`   | 409         | For optimistic locking implementations     |
| `Database`       | 500 / 404   | Raw details never leaked to clients        |
| `Internal`       | 500         |                                            |

5xx responses return a generic message; the real error is logged via `tracing`.

## Session Support

The `web` feature also exports a cookie-based session stack:

- `SessionMiddleware` – Actix middleware that loads / validates sessions
- `Identity` – extractable identity (`sub`, `username`, `email`)
- `SessionRepo` – persistence trait for session storage

Typical usage:

```rust
App::new()
    .wrap(SessionMiddleware::new(session_store).cookie_name("sid"))
    .service(/* routes that extract Identity */)
```

Missing or expired sessions yield `401 Unauthorized`.

## Architecture Overview

| Layer        | Responsibility                                      | Key Trait / Type          |
|--------------|-----------------------------------------------------|---------------------------|
| **ViewSet**  | HTTP extraction, routing, response mapping          | `ViewSet`, `DefaultViewSet` |
| **Service**  | Business rules, validation, transactional hooks     | `Service`, `DefaultService` |
| **Repository** | Data access, caching, query building              | `Repository`, `DefaultRepo` |
| **Entity**   | Static metadata + insert/update helpers             | `Entity` (+ derive macro) |

Request flow is strictly top-down; responses flow bottom-up. The ViewSet never talks to the database, and the Repository never performs validation.

## Cargo Features

| Feature   | Default | Description                                      |
|-----------|---------|--------------------------------------------------|
| `web`     | no      | Enables Actix-web integration, sessions, `ApiError`, etc. |

## Minimum Supported Rust Version

Edition 2024 is required (as declared in `Cargo.toml`).

## License

MIT

## Contributing

Issues and pull requests are welcome. When adding new entity capabilities, prefer extending the `Entity` trait metadata and the default `Repository` implementations so that existing resources continue to benefit automatically.

