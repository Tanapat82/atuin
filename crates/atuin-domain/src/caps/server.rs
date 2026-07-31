use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use super::{Capability, CapsBundle};

/// The result of comparing a client's echoed capability token against the server's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Negotiation {
    /// The client's token matches, or was absent -- serve the request.
    Current,
    /// The client presented a *differing* token -- its cached capabilities are stale.
    Stale,
}

/// Immutable set of capabilities a server advertises. Thread it as an [`Arc`].
#[derive(Debug)]
pub struct CapServer {
    /// Opaque version token (xxh3 of the canonical capability set), computed once.
    token: String,
    /// Pre-serialized capabilities document (a `CapabilitiesResponse` as JSON).
    body: String,
    /// The advertised capabilities, exposed for typed introspection via `caps`.
    caps: CapsBundle,
}

impl CapServer {
    /// Start building a capability set.
    pub fn builder() -> CapServerBuilder {
        CapServerBuilder::default()
    }

    /// The opaque version token this server advertises. Stable for a given capability set; the
    /// client echoes it back verbatim and never interprets it.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// The pre-serialized capabilities document, served verbatim by the capabilities endpoint.
    /// Deserializes into a [`crate::api::CapabilitiesResponse`].
    pub fn body(&self) -> &str {
        &self.body
    }

    /// The capabilities this server advertises, for typed introspection.
    pub fn caps(&self) -> &CapsBundle {
        &self.caps
    }

    /// Decide whether a request whose client echoed `known` is current.
    ///
    /// Absent (`None`) or matching tokens are [`Negotiation::Current`]; only a present *differing*
    /// token is [`Negotiation::Stale`]. A client that sends no token is therefore never rejected.
    pub fn negotiate(&self, known: Option<&str>) -> Negotiation {
        match known {
            Some(known) if known != self.token => Negotiation::Stale,
            _ => Negotiation::Current,
        }
    }
}

/// Builder for [`CapServer`]. Register capabilities with [`add`](Self::add), then
/// [`build`](Self::build). A builder with no `add` calls yields a server that advertises nothing.
#[derive(Debug, Default)]
pub struct CapServerBuilder {
    caps: CapsBundle,
}

impl CapServerBuilder {
    /// Advertise a capability. A later call with the same name overwrites the earlier value.
    #[allow(clippy::should_implement_trait)]
    pub fn add<C: Capability>(self, cap: C) -> Self {
        self.caps.add(cap);
        self
    }

    /// Finalize: compute the version token and pre-serialize the document. Cheap work done once.
    pub fn build(self) -> Arc<CapServer> {
        // A `BTreeMap` serializes its keys in sorted order, so the token is byte-identical on every
        // node running the same capability set.
        let wire = self.caps.to_wire();
        let canonical = serde_json::to_vec(&wire).expect("capability map serializes");
        let token = format!("{:016x}", xxhash_rust::xxh3::xxh3_64(&canonical));

        #[derive(Serialize)]
        struct Wire<'a> {
            version: &'a str,
            capabilities: &'a BTreeMap<String, Value>,
        }
        let body = serde_json::to_string(&Wire {
            version: &token,
            capabilities: &wire,
        })
        .expect("capabilities document serializes");

        Arc::new(CapServer {
            token,
            body,
            caps: self.caps,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::CapabilitiesResponse;
    use crate::caps::{CapabilitiesCap, Capability};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestCap {
        n: u32,
    }
    impl Capability for TestCap {
        const NAME: &'static str = "test/cap";
    }

    #[test]
    fn empty_server_advertises_nothing() {
        let caps = CapServer::builder().build();
        assert!(caps.caps().get::<TestCap>().is_none());
        assert!(caps.caps().get::<CapabilitiesCap>().is_none());
    }

    #[test]
    fn token_is_16_char_lowercase_hex() {
        let token = CapServer::builder().build().token().to_owned();
        assert_eq!(token.len(), 16);
        assert!(
            token
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    #[test]
    fn token_is_stable_for_the_same_set_and_changes_when_it_changes() {
        let empty_a = CapServer::builder().build();
        let empty_b = CapServer::builder().build();
        assert_eq!(empty_a.token(), empty_b.token());

        let with_cap = CapServer::builder().add(TestCap { n: 1 }).build();
        assert_ne!(empty_a.token(), with_cap.token());
        assert!(with_cap.caps().get::<TestCap>().is_some());
    }

    #[test]
    fn body_deserializes_into_the_client_response_shape() {
        let caps = CapServer::builder().build();
        let resp: CapabilitiesResponse = serde_json::from_str(caps.body()).unwrap();
        assert_eq!(resp.version, caps.token());
        assert!(resp.capabilities.is_empty());
    }

    #[test]
    fn body_with_a_capability_round_trips_into_the_client_response_shape() {
        let caps = CapServer::builder().add(TestCap { n: 7 }).build();
        let resp: CapabilitiesResponse = serde_json::from_str(caps.body()).unwrap();
        assert_eq!(resp.version, caps.token());
        assert_eq!(
            resp.capabilities.get("test/cap"),
            Some(&serde_json::json!({ "n": 7 }))
        );
    }

    #[test]
    fn negotiate_only_rejects_a_present_differing_token() {
        let caps = CapServer::builder().build();
        assert_eq!(caps.negotiate(None), Negotiation::Current);
        assert_eq!(caps.negotiate(Some(caps.token())), Negotiation::Current);
        assert_eq!(caps.negotiate(Some("deadbeefdeadbeef")), Negotiation::Stale);
    }
}
