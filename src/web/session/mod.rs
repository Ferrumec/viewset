mod middleware;
mod repo;
pub use middleware::{Identity, SessionMiddleware};
pub use repo::{NewSession, Session, SessionRepo};

pub fn create_session_middleware(
    db: sqlx::PgPool,
    cf: impl ferrumec::CacheFactory,
) -> SessionMiddleware {
    let repo = std::sync::Arc::new(crate::DefaultRepo::new(db, cf));
    SessionMiddleware::new(repo)
}
