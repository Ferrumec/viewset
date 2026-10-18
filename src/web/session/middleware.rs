use super::Session;
use actix_web::{
    Error, HttpMessage,
    body::MessageBody,
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    error,
};
use actix_web::{
    FromRequest, HttpRequest,
    dev::Payload,
    error::ErrorInternalServerError,
};
use ferrumec::{CacheFactory, Store};
use futures_util::future::LocalBoxFuture;
use std::{
    future::{ Ready, ready},
    rc::Rc,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
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
    store: Arc<dyn Store<Uuid, Session>>,
    cookie_name: String,
}

impl SessionMiddleware {
    /// Create a `SessionMiddleware` backed by `store`.
    ///
    /// A request with no session cookie, an invalid `Uuid`, a missing store
    /// entry, or an expired session is rejected with `401 Unauthorized`.
    /// The cookie name defaults to `"session"`.
    pub fn new(cf: impl CacheFactory) -> Self {
        Self {
            store: cf.new_cache("sessions_items", Duration::from_mins(30)),
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
    store: Arc<dyn Store<Uuid, Session>>,
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
            match req.cookie(&cookie_name) {
                Some(cookie) => {
                    let id = match Uuid::parse_str(cookie.value()) {
                        Ok(id) => id,
                        Err(_) => {
                            return Err(error::ErrorUnauthorized("no session"));
                        }
                    };
                    let session_data = match store.get(&id).await? {
                        Some(data) => data,
                        None => {
                            return Err(error::ErrorUnauthorized("no session"));
                        }
                    };
                    // Reject expired sessions
                    let IdentityResult::Active(identity) = session_data.identify() else {
                        return Err(error::ErrorUnauthorized("session expired"));
                    };
                    req.extensions_mut().insert(identity);
                    let res = service.call(req).await?;
                    Ok(res)
                }
                None => {
                    return Err(error::ErrorUnauthorized("no session"));
                }
            }
        })
    }
}
