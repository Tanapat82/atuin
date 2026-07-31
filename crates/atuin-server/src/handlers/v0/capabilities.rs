//! The capability endpoint and negotiation middleware now live in atuin-domain behind its `axum`
//! feature (`atuin_domain::caps::axum`); re-exported here so the router wiring is unchanged, and
//! exercised against a real axum router by the tests below.

pub use atuin_domain::caps::axum::{get, negotiate};

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use atuin_domain::caps::CapServer;
    use atuin_domain::caps::http::{AVAILABLE_HEADER, KNOWN_HEADER};
    use axum::{
        Router,
        body::Body,
        http::{HeaderValue, Request, StatusCode},
        routing::get as axum_get,
    };
    use rstest::{fixture, rstest};
    use tower::ServiceExt; // oneshot

    /// An empty capability set -- advertises nothing, but still issues a stable token.
    #[fixture]
    fn caps() -> Arc<CapServer> {
        CapServer::builder().build()
    }

    #[rstest]
    #[tokio::test]
    async fn endpoint_serves_the_document(caps: Arc<CapServer>) {
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

    fn negotiating_app(caps: Arc<CapServer>) -> Router {
        Router::new()
            .route("/probe", axum_get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(caps, negotiate))
    }

    #[rstest]
    #[tokio::test]
    async fn absent_known_header_passes(caps: Arc<CapServer>) {
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
    async fn matching_token_passes(caps: Arc<CapServer>) {
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
    async fn non_utf8_known_header_is_treated_as_absent_and_passes(caps: Arc<CapServer>) {
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
    async fn stale_token_rejects_with_available_header(caps: Arc<CapServer>) {
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
