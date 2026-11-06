use actix_web::{
    Error, HttpMessage,
    body::MessageBody,
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    error,
};
use actix_web::{FromRequest, HttpRequest, dev::Payload, error::ErrorInternalServerError};
use ferrumec::Store;
use futures_util::future::LocalBoxFuture;
use std::{
    future::{Ready, ready},
    rc::Rc,
    sync::Arc,
    task::{Context, Poll},
};
use uuid::Uuid;

#[derive(Clone)]
pub struct Identity {
    pub sub: Uuid,
    pub username: String,
    pub email: String,
}

impl FromRequest for Identity {
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        // Prefer the Filters already constructed by middleware.
        if let Some(filters) = req.extensions().get::<Identity>() {
            return Box::pin(ready(Ok(filters.clone())));
        }
        tracing::error!("Session middleware not wrapped for {}", req.path());
        Box::pin(ready(Err(ErrorInternalServerError("Configuration Error"))))
    }
}

pub enum IdentityResult {
    Active(Identity),
    Expired,
}

/// Middleware factory for cookie-based session storage.
///
/// Construct with [`SessionMiddleware::new`]. Missing, invalid, or expired
/// session cookies are rejected with `401 Unauthorized`. Customise the cookie
/// name with [`cookie_name`](Self::cookie_name).
pub struct SessionMiddleware {
    store: Arc<dyn Store<Uuid, Identity>>,
    cookie_name: String,
}

impl SessionMiddleware {
    /// Create a `SessionMiddleware` backed by `store`.
    ///
    /// A request with no session cookie, an invalid `Uuid`, a missing store
    /// entry, or an expired session is rejected with `401 Unauthorized`.
    /// The cookie name defaults to `"session"`.
    pub fn new(store: Arc<dyn Store<Uuid, Identity>>) -> Self {
        Self {
            store,
            cookie_name: "session".into(),
        }
    }

    /// Override the session cookie name (default: `"session"`).
    pub fn cookie_name(mut self, name: impl Into<String>) -> Self {
        self.cookie_name = name.into();
        self
    }
}

impl<S, B> Transform<S, ServiceRequest> for SessionMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = SessionMiddlewareService<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;
    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(SessionMiddlewareService {
            service: service.into(),
            store: self.store.clone(),
            cookie_name: self.cookie_name.clone(),
        }))
    }
}

/// The inner service produced by [`SessionMiddleware`].
pub struct SessionMiddlewareService<R> {
    service: Rc<R>,
    store: Arc<dyn Store<Uuid, Identity>>,
    cookie_name: String,
}

impl<S, B> Service<ServiceRequest> for SessionMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let store = self.store.clone();
        let cookie_name = self.cookie_name.clone();
        let service = self.service.clone();

        Box::pin(async move {
            let path = req.path().to_owned();

            let cookie = req.cookie(&cookie_name).ok_or_else(|| {
                tracing::warn!("no session cookie on request to {path}");
                error::ErrorUnauthorized("no session")
            })?;

            let id = Uuid::parse_str(cookie.value()).map_err(|e| {
                tracing::warn!("invalid session id on request to {path}: {e}");
                error::ErrorUnauthorized("no session")
            })?;

            let identity = store
                .get(&id)
                .await
                .map_err(|e| {
                    tracing::error!("session store failure on request to {path}: {e}");
                    error::ErrorInternalServerError("server failed to authenticate")
                })?
                .ok_or_else(|| {
                    tracing::warn!("session {id} not found for request to {path}");
                    error::ErrorUnauthorized("no session")
                })?;

            req.extensions_mut().insert(identity);
            service.call(req).await
        })
    }
}
