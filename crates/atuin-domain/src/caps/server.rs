use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use super::Capability;

/// The result of comparing a client's echoed capability token against the server's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Negotiation {
    /// The client's token matches, or was absent -- serve the request.
    Current,
    /// The client presented a *differing* token -- its cached capabilities are stale.
    Stale,
}

/// Immutable, cheaply-cloneable set of capabilities a server advertises.
#[derive(Debug, Clone)]
pub struct CapServer {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    /// Opaque version token (xxh3 of the canonical capability set), computed once.
    token: String,
    /// Pre-serialized capabilities document (a `CapabilitiesResponse` as JSON).
    body: String,
    /// The advertised capabilities, kept for introspection (`advertises`).
    caps: BTreeMap<String, Value>,
}

impl CapServer {
    /// Start building a capability set.
    pub fn builder() -> CapServerBuilder {
        CapServerBuilder::default()
    }

    /// The opaque version token this server advertises. Stable for a given capability set; the
    /// client echoes it back verbatim and never interprets it.
    pub fn token(&self) -> &str {
        &self.inner.token
    }

    /// The pre-serialized capabilities document, served verbatim by the capabilities endpoint.
    /// Deserializes into a [`super::CapabilitiesResponse`].
    pub fn body(&self) -> &str {
        &self.inner.body
    }

    /// Whether this server advertises the capability with the given wire name.
    pub fn advertises(&self, name: &str) -> bool {
        self.inner.caps.contains_key(name)
    }

    /// Decide whether a request whose client echoed `known` is current.
    ///
    /// Absent (`None`) or matching tokens are [`Negotiation::Current`]; only a present *differing*
    /// token is [`Negotiation::Stale`]. A client that sends no token is therefore never rejected.
    pub fn negotiate(&self, known: Option<&str>) -> Negotiation {
        match known {
            Some(known) if known != self.inner.token => Negotiation::Stale,
            _ => Negotiation::Current,
        }
    }
}

/// Builder for [`CapServer`]. Register capabilities with [`can`](Self::can), then
/// [`build`](Self::build). A builder with no `can` calls yields a server that advertises nothing.
#[derive(Debug, Default)]
pub struct CapServerBuilder {
    caps: BTreeMap<String, Value>,
}

impl CapServerBuilder {
    /// Advertise a capability. A later call with the same name overwrites the earlier value.
    pub fn add<C: Capability>(mut self, cap: C) -> Self {
        let value =
            serde_json::to_value(cap).expect("a capability value must be JSON-serializable");
        self.caps.insert(C::NAME.to_string(), value);
        self
    }

    /// Finalize: compute the version token and pre-serialize the document. Cheap work done once.
    pub fn build(self) -> CapServer {
        // A `BTreeMap` serializes its keys in sorted order, so the token is byte-identical on every
        // node running the same capability set.
        let canonical = serde_json::to_vec(&self.caps).expect("capability map serializes");
        let token = format!("{:016x}", xxhash_rust::xxh3::xxh3_64(&canonical));

        #[derive(Serialize)]
        struct Wire<'a> {
            version: &'a str,
            capabilities: &'a BTreeMap<String, Value>,
        }
        let body = serde_json::to_string(&Wire {
            version: &token,
            capabilities: &self.caps,
        })
        .expect("capabilities document serializes");

        CapServer {
            inner: Arc::new(Inner {
                token,
                body,
                caps: self.caps,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::{CapabilitiesCap, CapabilitiesResponse, Capability};
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
        assert!(!caps.advertises("test/cap"));
        assert!(!caps.advertises(CapabilitiesCap::NAME));
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
        assert!(with_cap.advertises("test/cap"));
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
