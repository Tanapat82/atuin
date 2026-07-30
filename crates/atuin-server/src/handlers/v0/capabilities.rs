//! The capabilities endpoint and the capability-negotiation middleware.
//!
//! Both take the [`CapServer`] as axum state, so they are independent of the database and of
//! `AppState`. In the real router the endpoint gets it as its own router state via `with_state`,
//! and the middleware is given it directly via `from_fn_with_state`.

use atuin_domain::caps::http::{AVAILABLE_HEADER, KNOWN_HEADER};
use atuin_domain::caps::{CapServer, Negotiation};
use axum::{
    extract::{Request, State},
    http::{HeaderName, HeaderValue, StatusCode, header::CONTENT_TYPE},
    middleware::Next,
    response::{IntoResponse, Response},
};

/// `GET /api/v0/capabilities` -- serve the pre-serialized capability document.
pub async fn get(State(caps): State<CapServer>) -> Response {
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
pub async fn negotiate(State(caps): State<CapServer>, request: Request, next: Next) -> Response {
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

#[cfg(test)]
mod tests {
    use super::*;
    use atuin_domain::caps::CapServer;
    use atuin_domain::caps::http::{AVAILABLE_HEADER, KNOWN_HEADER};
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        routing::get as axum_get,
    };
    use rstest::{fixture, rstest};
    use tower::ServiceExt; // oneshot

    /// An empty capability set -- advertises nothing, but still issues a stable token.
    #[fixture]
    fn caps() -> CapServer {
        CapServer::builder().build()
    }

    #[rstest]
    #[tokio::test]
    async fn endpoint_serves_the_document(caps: CapServer) {
        let app: Router = Router::new()
            .route("/api/v0/capabilities", axum_get(get))
            .with_state(caps.clone());

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v0/capabilities")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(bytes.as_ref(), caps.body().as_bytes());
    }

    fn negotiating_app(caps: CapServer) -> Router {
        Router::new()
            .route("/probe", axum_get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(caps, negotiate))
    }

    #[rstest]
    #[tokio::test]
    async fn absent_known_header_passes(caps: CapServer) {
        let resp = negotiating_app(caps)
            .oneshot(
                Request::builder()
                    .uri("/probe")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[rstest]
    #[tokio::test]
    async fn matching_token_passes(caps: CapServer) {
        let resp = negotiating_app(caps.clone())
            .oneshot(
                Request::builder()
                    .uri("/probe")
                    .header(KNOWN_HEADER, caps.token())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[rstest]
    #[tokio::test]
    async fn non_utf8_known_header_is_treated_as_absent_and_passes(caps: CapServer) {
        // `to_str().ok()` turns invalid UTF-8 into `None`, i.e. "no token known" -- not a 412,
        // and not a panic.
        let value = HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap();
        let resp = negotiating_app(caps)
            .oneshot(
                Request::builder()
                    .uri("/probe")
                    .header(KNOWN_HEADER, value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[rstest]
    #[tokio::test]
    async fn stale_token_rejects_with_available_header(caps: CapServer) {
        let resp = negotiating_app(caps.clone())
            .oneshot(
                Request::builder()
                    .uri("/probe")
                    .header(KNOWN_HEADER, "deadbeefdeadbeef")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::PRECONDITION_FAILED);
        assert_eq!(
            resp.headers()
                .get(AVAILABLE_HEADER)
                .unwrap()
                .to_str()
                .unwrap(),
            caps.token()
        );
    }
}
