use super::entity::Entity;
use super::error::{ApiError, ApiResult};
use super::pagination::{PaginationParams, SortDirection};
use super::sql::SqlType;
use crate::no_cache::NoCache;
use actixutils::{Filters, Store};
use async_trait::async_trait;
use moka::future::Cache;
use sqlx::{PgPool, Postgres, QueryBuilder, Transaction};
use std::sync::Arc;
/// Database access only: no validation, authorization, or business rules.
/// Every method has a default implementation built from `Entity` metadata
/// via a dynamic `QueryBuilder`; override any of them when the generated
/// SQL isn't good enough (complex joins, window functions, etc.).
#[async_trait]
pub trait Repository: Send + Sync {
    type Entity: Entity;

    fn database(&self) -> &PgPool;

    fn cache(
        &self,
    ) -> Arc<dyn Store<<<Self as Repository>::Entity as Entity>::Id, Self::Entity> + Send + Sync>
    {
        NoCache::new()
    }

    fn list_cache(&self) -> Arc<dyn Store<u64, (Vec<Self::Entity>, i64)> + Send + Sync> {
        NoCache::new()
    }

    /// Begin a transaction against this repository's pool. Used by the
    /// default `Service::create`/`update`/`delete` implementations so a
    /// mutation and its `before_*`/`after_*` hooks run atomically — if a
    /// hook (or the write itself) fails, everything rolls back together.
    async fn transaction(&self) -> ApiResult<Transaction<'_, Postgres>> {
        Ok(self.database().begin().await?)
    }

    async fn list(&self, query: &Filters) -> ApiResult<(Vec<Self::Entity>, i64)> {
        let key = hash_params(&query.0);
        if let Ok(Some((items, total))) = self.list_cache().get(&key).await {
            return Ok((items, total));
        }
        let pagination = PaginationParams::from_query(query);
        let e = <Self::Entity as Entity>::TABLE;

        let mut count_qb: QueryBuilder<Postgres> =
            QueryBuilder::new(format!("SELECT COUNT(*) FROM {e}"));
        let mut select_qb: QueryBuilder<Postgres> = QueryBuilder::new(format!(
            "SELECT {} FROM {e}",
            <Self::Entity as Entity>::COLUMNS.join(", ")
        ));

        // `select_qb` and `count_qb` are two independent statements and
        // must track their own WHERE state separately — sharing one flag
        // between them caused a bare `AND` with no preceding `WHERE` on
        // `count_qb` whenever a soft-delete column or any filter/search
        // param was involved, which Postgres rejects as a syntax error.
        let mut select_has_where = false;
        let mut count_has_where = false;
        push_soft_delete_clause::<Self::Entity>(&mut select_qb, &mut select_has_where);
        push_soft_delete_clause::<Self::Entity>(&mut count_qb, &mut count_has_where);
        push_filters::<Self::Entity>(&mut select_qb, query, &mut select_has_where);
        push_filters::<Self::Entity>(&mut count_qb, query, &mut count_has_where);

        if let Some(sort) = &query.get("sort") {
            let clauses: Vec<String> = PaginationParams::parse_sort(sort)
                .into_iter()
                .filter(|(field, _)| <Self::Entity as Entity>::SORTABLE.contains(&field.as_str()))
                .map(|(field, dir)| {
                    format!(
                        "{field} {}",
                        match dir {
                            SortDirection::Asc => "ASC",
                            SortDirection::Desc => "DESC",
                        }
                    )
                })
                .collect();
            if !clauses.is_empty() {
                select_qb.push(" ORDER BY ").push(clauses.join(", "));
            }
        }

        select_qb
            .push(" LIMIT ")
            .push_bind(pagination.limit as i64)
            .push(" OFFSET ")
            .push_bind(pagination.offset as i64);

        let items = select_qb
            .build_query_as::<Self::Entity>()
            .fetch_all(self.database())
            .await?;
        let total: i64 = count_qb
            .build_query_scalar()
            .fetch_one(self.database())
            .await?;

        Ok((items, total))
    }

    /// Cache-aside read: a hit returns straight from `cache()` without
    /// touching the database; a miss falls through to the row fetch and
    /// populates the cache before returning. Safe to populate
    /// unconditionally here — unlike the `_in_tx` write paths below,
    /// there's no open transaction that could still roll back and turn
    /// this into a phantom entry.
    async fn retrieve(&self, id: &<Self::Entity as Entity>::Id) -> ApiResult<Self::Entity> {
        if let Ok(Some(hit)) = self.cache().get(&id).await {
            return Ok(hit);
        }
        let entity = retrieve_row::<_, Self::Entity>(self.database(), id).await?;
        if let Err(e) = self.cache().set(&entity.id(), entity.clone()).await {
            tracing::warn!("failed to set cache: {e}");
        };
        Ok(entity)
    }

    /// Runs as its own auto-committed statement (no explicit `BEGIN`), so
    /// by the time this returns the row is durably written and it's safe
    /// to populate the cache with it directly (write-through).
    async fn create(&self, dto: <Self::Entity as Entity>::CreateDto) -> ApiResult<Self::Entity> {
        let entity = Self::Entity::insert(dto, self.database()).await?;
        if let Err(e) = self.cache().set(&entity.id(), entity.clone()).await {
            tracing::warn!("failed to set cache: {e}");
        };
        Ok(entity)
    }

    /// Same as `create`, but runs against an already-open transaction so
    /// the insert can be committed or rolled back together with whatever
    /// a `Service`'s `before_create`/`after_create` hooks do in that same
    /// transaction.
    ///
    /// Deliberately does **not** touch the cache. The row isn't committed
    /// yet at this point — if a later hook in the same transaction fails
    /// and the caller rolls back, a cache entry set here would describe an
    /// id that never actually existed. Once the transaction commits, a
    /// subsequent `retrieve()` will populate the cache normally via the
    /// cache-aside path above.

    async fn create_in_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        dto: <Self::Entity as Entity>::CreateDto, // now by value, matches create()
    ) -> ApiResult<Self::Entity> {
        Ok(Self::Entity::insert(dto, &mut **tx).await?)
    }

    /// Auto-committed like `create`, so the write-through cache update
    /// below is ordered safely after the durable write.
    async fn update(
        &self,
        id: &<Self::Entity as Entity>::Id,
        dto: <Self::Entity as Entity>::UpdateDto, // now by value
    ) -> ApiResult<Self::Entity> {
        let entity = Self::Entity::update(id, dto, self.database())
            .await?
            .ok_or(ApiError::NotFound)?;
        if let Err(e) = self.cache().set(&entity.id(), entity.clone()).await {
            tracing::warn!("failed to set cache: {e}");
        };
        Ok(entity)
    }

    /// Transactional counterpart to `update`, see `create_in_tx`.
    ///
    /// Invalidates (rather than repopulates) the cache entry: unlike
    /// `create_in_tx`, there's a previously-cached value to worry about
    /// here, and holding onto a stale one is worse than dropping it.
    /// Overwriting it with the *new* row would be worse still — same
    /// rollback risk as `create_in_tx` — so this drops the entry and lets
    /// the next `retrieve()` repopulate it post-commit.
    ///
    /// Known race: a concurrent `retrieve()` between this invalidation and
    /// the caller's `tx.commit()` will still observe the pre-update row
    /// (Postgres read-committed semantics) and re-cache *that*, leaving a
    /// stale entry after commit. Closing that window fully needs a
    /// post-commit invalidation from the `Service` layer, which owns the
    /// commit point — worth revisiting if strict read-after-write
    /// consistency through the cache becomes a requirement.
    async fn update_in_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: &<Self::Entity as Entity>::Id,
        dto: <Self::Entity as Entity>::UpdateDto,
    ) -> ApiResult<Self::Entity> {
        let entity = Self::Entity::update(id, dto, &mut **tx)
            .await?
            .ok_or(ApiError::NotFound)?;
        if let Err(e) = self.cache().delete(id).await {
            tracing::error!("failed to invalidate cache: {e}");
        };
        Ok(entity)
    }

    /// Auto-committed like `create`/`update`, so invalidating the cache
    /// entry here is correctly ordered after the durable delete.
    async fn delete(&self, id: &<Self::Entity as Entity>::Id) -> ApiResult<()> {
        delete_row::<_, Self::Entity>(self.database(), id).await?;
        if let Err(e) = self.cache().delete(id).await {
            tracing::error!("failed to invalidate cache: {e}");
        };
        Ok(())
    }

    /// Transactional counterpart to `delete`, see `create_in_tx` and
    /// `update_in_tx` for why this invalidates rather than caching
    /// anything, and for the same pre-commit race `update_in_tx` has.
    async fn delete_in_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: &<Self::Entity as Entity>::Id,
    ) -> ApiResult<()> {
        delete_row::<_, Self::Entity>(&mut **tx, id).await?;
        if let Err(e) = self.cache().delete(id).await {
            tracing::error!("failed to invalidate cache: {e}");
        };
        Ok(())
    }

    async fn exists(&self, id: &<Self::Entity as Entity>::Id) -> ApiResult<bool> {
        let e = <Self::Entity as Entity>::TABLE;
        let pk = <Self::Entity as Entity>::PK_COLUMN;
        let mut qb: QueryBuilder<Postgres> =
            QueryBuilder::new(format!("SELECT EXISTS(SELECT 1 FROM {e} WHERE {pk} = "));
        qb.push_bind(id);
        qb.push(")");

        let exists: bool = qb.build_query_scalar().fetch_one(self.database()).await?;
        Ok(exists)
    }

    async fn count(&self) -> ApiResult<i64> {
        let e = <Self::Entity as Entity>::TABLE;
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(format!("SELECT COUNT(*) FROM {e}"));
        Ok(qb.build_query_scalar().fetch_one(self.database()).await?)
    }
}

/// Whether `E`'s soft-delete column (if any) is declared as `Bool` in
/// `Entity::FIELDS`. Anything else (nullable timestamp being the common
/// case) is treated as timestamp-flavored soft delete.
fn soft_delete_is_bool<E: Entity>(col: &str) -> bool {
    E::FIELDS
        .iter()
        .any(|(name, ty)| *name == col && *ty == SqlType::Bool)
}

fn push_soft_delete_clause<E: Entity>(qb: &mut QueryBuilder<Postgres>, has_where: &mut bool) {
    if let Some(col) = E::SOFT_DELETE_COLUMN {
        qb.push(if *has_where { " AND " } else { " WHERE " });
        if soft_delete_is_bool::<E>(col) {
            // Boolean flag: NULL and false both mean "not deleted".
            qb.push(format!("{col} IS NOT TRUE"));
        } else {
            qb.push(format!("{col} IS NULL"));
        }
        *has_where = true;
    }
}

fn push_filters<E: Entity>(qb: &mut QueryBuilder<Postgres>, query: &Filters, has_where: &mut bool) {
    for (field, value) in query.iter() {
        if !E::FILTERABLE.contains(&field.as_str()) {
            continue; // silently ignore unknown/forbidden filter keys
        }
        qb.push(if *has_where { " AND " } else { " WHERE " });
        qb.push(format!("{field} = "));
        qb.push_bind(value.clone());
        *has_where = true;
    }
    if let Some(search) = &query.get("search")
        && !search.is_empty()
        && !E::SEARCHABLE.is_empty()
    {
        qb.push(if *has_where { " AND (" } else { " WHERE (" });
        let pattern = format!("%{search}%");
        for (i, field) in E::SEARCHABLE.iter().enumerate() {
            if i > 0 {
                qb.push(" OR ");
            }
            qb.push(format!("{field} ILIKE "));
            qb.push_bind(pattern.clone());
        }
        qb.push(")");
        *has_where = true;
    }
}

/// Generic single-row fetch by primary key, usable against either a pool
/// or an open transaction.
async fn retrieve_row<'e, E, Ent>(exec: E, id: &Ent::Id) -> ApiResult<Ent>
where
    E: sqlx::postgres::PgExecutor<'e>,
    Ent: Entity,
{
    let table = Ent::TABLE;
    let pk = Ent::PK_COLUMN;
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(format!(
        "SELECT {} FROM {table} WHERE {pk} = ",
        Ent::COLUMNS.join(", ")
    ));
    qb.push_bind(id);
    let mut has_where = true;
    push_soft_delete_clause::<Ent>(&mut qb, &mut has_where);

    qb.build_query_as::<Ent>()
        .fetch_optional(exec)
        .await?
        .ok_or(ApiError::NotFound)
}

/// Generic DELETE (or soft delete) by primary key, usable against either a
/// pool or an open transaction. Soft delete now respects the column's
/// actual type: `SET col = true` for a boolean flag, `SET col = now()`
/// for the more common nullable-timestamp column — previously this
/// always wrote `now()`, which fails against a boolean column.
async fn delete_row<'e, E, Ent>(exec: E, id: &Ent::Id) -> ApiResult<()>
where
    E: sqlx::postgres::PgExecutor<'e>,
    Ent: Entity,
{
    let table = Ent::TABLE;
    let pk = Ent::PK_COLUMN;

    let mut qb: QueryBuilder<Postgres> = if let Some(col) = Ent::SOFT_DELETE_COLUMN {
        if soft_delete_is_bool::<Ent>(col) {
            QueryBuilder::new(format!("UPDATE {table} SET {col} = true WHERE {pk} = "))
        } else {
            QueryBuilder::new(format!("UPDATE {table} SET {col} = now() WHERE {pk} = "))
        }
    } else {
        QueryBuilder::new(format!("DELETE FROM {table} WHERE {pk} = "))
    };
    qb.push_bind(id);

    let res = qb.build().execute(exec).await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

use std::hash::{Hash, Hasher};

pub fn hash_params(params: &std::collections::HashMap<String, String>) -> u64 {
    let mut entries: Vec<_> = params.iter().collect();

    entries.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));

    let mut hasher = std::hash::DefaultHasher::new();

    for (key, value) in entries {
        key.hash(&mut hasher);
        value.hash(&mut hasher);
    }

    hasher.finish()
}

pub struct DefaultRepo<E: Entity> {
    db: PgPool,
    cache: Arc<Cache<E::Id, E>>,
    list_cache: Arc<Cache<u64, (Vec<E>, i64)>>,
}

impl<E: Entity> From<PgPool> for DefaultRepo<E> {
    fn from(db: PgPool) -> DefaultRepo<E> {
        Self {
            db,
            cache: Arc::new(Cache::new(1000)),
            list_cache: Arc::new(Cache::new(1000)),
        }
    }
}

impl<E: Entity> Repository for DefaultRepo<E> {
    type Entity = E;
    fn cache(&self) -> Arc<dyn Store<E::Id, E> + Send + Sync> {
        self.cache.clone()
    }
    fn list_cache(&self) -> Arc<dyn Store<u64, (Vec<E>, i64)> + Send + Sync> {
        self.list_cache.clone()
    }
    fn database(&self) -> &PgPool {
        &self.db
    }
}
