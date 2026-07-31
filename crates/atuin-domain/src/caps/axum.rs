//! Axum integration for capability negotiation: the capabilities endpoint and the negotiation
//! middleware. Gated behind the `axum` feature.
//!
//! Both take the [`CapServer`] as axum state, so they are independent of any database or app
//! state. In a router the endpoint gets it via `with_state` and the middleware via
//! `from_fn_with_state`.

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{HeaderName, HeaderValue, StatusCode, header::CONTENT_TYPE},
    middleware::Next,
    response::{IntoResponse, Response},
};

use super::http::{AVAILABLE_HEADER, KNOWN_HEADER};
use super::{CapServer, Negotiation};

/// `GET /api/v0/capabilities` -- serve the pre-serialized capability document.
pub async fn get(State(caps): State<Arc<CapServer>>) -> Response {
    (
        [(CONTENT_TYPE, HeaderValue::from_static("application/json"))],
        caps.body().to_owned(),
    )
        .into_response()
}

/// Capability-negotiation middleware for the sync API.
///
/// Rejects with `412` + `X-Atuin-Capabilities-Available` **only** when the request carries
/// `X-Atuin-Capabilities-Known` with a token differing from ours. Absent or matching tokens pass
/// straight through, so pre-capabilities clients are never affected. A non-UTF-8 known header is
/// treated as absent.
pub async fn negotiate(
    State(caps): State<Arc<CapServer>>,
    request: Request,
    next: Next,
) -> Response {
    let known = request
        .headers()
        .get(KNOWN_HEADER)
        .and_then(|value| value.to_str().ok());

    match caps.negotiate(known) {
        Negotiation::Current => next.run(request).await,
        Negotiation::Stale => {
            let mut response =
                (StatusCode::PRECONDITION_FAILED, "capabilities out of date").into_response();
            // The token is precomputed ASCII hex, so this never fails; guard rather than panic.
            if let Ok(value) = HeaderValue::from_str(caps.token()) {
                response
                    .headers_mut()
                    .insert(HeaderName::from_static(AVAILABLE_HEADER), value);
            }
            response
        }
    }
}
