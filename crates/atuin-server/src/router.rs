use atuin_domain::caps::CapServer;
use atuin_domain::api::{ATUIN_CARGO_VERSION, ATUIN_HEADER_VERSION, ErrorResponse};
use axum::{
    Router,
    extract::{FromRef, FromRequestParts, Request},
    http::{self, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post},
};
use eyre::Result;
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;

use super::handlers;
use crate::{
    handlers::{ErrorResponseStatus, RespExt},
    metrics,
    settings::Settings,
};
use atuin_server_database::{Database, DbError, models::User};

pub struct UserAuth(pub User);

impl<DB: Send + Sync> FromRequestParts<AppState<DB>> for UserAuth
where
    DB: Database,
{
    type Rejection = ErrorResponseStatus<'static>;

    async fn from_request_parts(
        req: &mut Parts,
        state: &AppState<DB>,
    ) -> Result<Self, Self::Rejection> {
        let auth_header = req
            .headers
            .get(http::header::AUTHORIZATION)
            .ok_or_else(|| {
                ErrorResponse::reply("missing authorization header")
                    .with_status(http::StatusCode::BAD_REQUEST)
            })?;
        let auth_header = auth_header.to_str().map_err(|_| {
            ErrorResponse::reply("invalid authorization header encoding")
                .with_status(http::StatusCode::BAD_REQUEST)
        })?;
        let (typ, token) = auth_header.split_once(' ').ok_or_else(|| {
            ErrorResponse::reply("invalid authorization header encoding")
                .with_status(http::StatusCode::BAD_REQUEST)
        })?;

        if typ != "Token" {
            return Err(
                ErrorResponse::reply("invalid authorization header encoding")
                    .with_status(http::StatusCode::BAD_REQUEST),
            );
        }

        let user = state
            .database
            .get_session_user(token)
            .await
            .map_err(|e| match e {
                DbError::NotFound => ErrorResponse::reply("session not found")
                    .with_status(http::StatusCode::FORBIDDEN),
                DbError::Other(e) => {
                    tracing::error!(error = ?e, "could not query user session");
                    ErrorResponse::reply("could not query user session")
                        .with_status(http::StatusCode::INTERNAL_SERVER_ERROR)
                }
            })?;

        Ok(UserAuth(user))
    }
}

async fn teapot() -> impl IntoResponse {
    // This used to return 418: 🫖
    // Much as it was fun, it wasn't as useful or informative as it should be
    (http::StatusCode::NOT_FOUND, "404 not found")
}

async fn clacks_overhead(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;

    let gnu_terry_value = "GNU Terry Pratchett, Kris Nova";
    let gnu_terry_header = "X-Clacks-Overhead";

    response
        .headers_mut()
        .insert(gnu_terry_header, gnu_terry_value.parse().unwrap());
    response
}

/// Ensure that we only try and sync with clients on the same major version
async fn semver(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(ATUIN_HEADER_VERSION, ATUIN_CARGO_VERSION.parse().unwrap());

    response
}

#[derive(Clone)]
pub struct AppState<DB: Database> {
    pub database: DB,
    pub settings: Settings,
    pub caps: CapServer,
}

impl<DB: Database> FromRef<AppState<DB>> for CapServer {
    fn from_ref(state: &AppState<DB>) -> Self {
        state.caps.clone()
    }
}

/// The capability set the self-hosted server advertises: empty. Absence of
/// `sh.atuin.server/records.bundle` is how clients learn packfile bundling is unsupported.
pub(crate) fn server_caps() -> CapServer {
    CapServer::builder().build()
}

pub fn router<DB: Database>(database: DB, settings: Settings) -> Router {
    let caps = server_caps();

    // Everything except the health check negotiates capabilities: a capability-aware client that
    // presents a stale token gets a 412 so it refreshes before we act on its request. Clients that
    // send no `X-Atuin-Capabilities-Known` header -- browsers, monitors, pre-capabilities clients --
    // pass straight through, so negotiation only ever affects atuin clients whose view of the
    // server is out of date.
    let negotiated = Router::new()
        .route("/", get(handlers::index))
        .route("/user/{username}", get(handlers::user::get))
        .route("/account", delete(handlers::user::delete))
        .route("/account/password", patch(handlers::user::change_password))
        .route("/register", post(handlers::user::register))
        .route("/login", post(handlers::user::login))
        .route("/api/v0/me", get(handlers::v0::me::get))
        .route("/api/v0/record", post(handlers::v0::record::post))
        .route("/api/v0/record", get(handlers::v0::record::index))
        .route("/api/v0/record/next", get(handlers::v0::record::next))
        .route("/api/v0/store", delete(handlers::v0::store::delete))
        .layer(axum::middleware::from_fn_with_state(
            caps.clone(),
            handlers::v0::capabilities::negotiate,
        ));

    // Two routes never negotiate, for different reasons. The capabilities endpoint must stay
    // ungated so a client with a stale token can always refresh here without a 412; the health
    // check must always answer for load balancers and monitors, independent of any capability
    // state.
    let unnegotiated = Router::new()
        .route("/api/v0/capabilities", get(handlers::v0::capabilities::get))
        .route("/healthz", get(handlers::health::health_check));

    let routes = unnegotiated.merge(negotiated);

    let path = settings.path.as_str();
    let routes = if path.is_empty() {
        routes
    } else {
        Router::new().nest(path, routes)
    };

    routes
        .fallback(teapot)
        .with_state(AppState {
            database,
            settings,
            caps,
        })
        .layer(
            ServiceBuilder::new()
                .layer(axum::middleware::from_fn(clacks_overhead))
                .layer(TraceLayer::new_for_http())
                .layer(axum::middleware::from_fn(metrics::track_metrics))
                .layer(axum::middleware::from_fn(semver)),
        )
}
