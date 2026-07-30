//! Capabilities system used by atuin.
//!
//! A node advertises capabilities about itself and, if it is a client, can read the server's.
//! The asymmetry is deliberate: capabilities are announced by *hosting* an endpoint. The server
//! hosts one; the client does not, so the server has nowhere to query the client's capabilities
//! from (yet).
//!
//! - [`CapServer`] -- an immutable, build-once set of a server's own capabilities with a
//!   precomputed version token; see [`CapServer::negotiate`].
//! - [`CapClient`] -- own capabilities, plus [`CapClient::refresh`] to pull the server's over a
//!   borrowed [`reqwest::Client`] and [`CapClient::server_support`] to read them back.

use parking_lot::RwLock;
use std::{any::Any, borrow::Borrow, collections::HashMap, fmt};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub mod http;

mod client;
mod middleware;
mod server;

pub use client::{CapClient, ServerSupportError};
pub use middleware::{CapMiddleware, CapabilitiesExt};
pub use server::{CapServer, CapServerBuilder, Negotiation};

/// A capability is always indexed by a String key.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, derive_more::AsRef)]
struct CapKey(String);

impl Borrow<str> for CapKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// A capability which two peers may negotiate.
pub trait Capability: Serialize + DeserializeOwned + Send + Sync + 'static {
    /// The name this capability is indexed by on the wire, eg `sh.atuin.server/records.batch`.
    const NAME: &'static str;
}

/// The capabilities a server advertises, as returned from its capabilities endpoint.
#[derive(Debug, Serialize, Deserialize)]
pub struct CapabilitiesResponse {
    /// An opaque capability token issued by the server.
    ///
    /// The server is free to choose its own scheme (a hash, a monotonic counter, a UUID, ...); the
    /// client stores and echoes this value back verbatim in `X-Atuin-Capabilities-Known` and never
    /// interprets it. If the client and server disagree on this token, none of the capabilities are
    /// to be assumed.
    pub version: String,

    /// The list of capabilities this server supports, as a map of capability name to its value.
    pub capabilities: HashMap<String, serde_json::Value>,
}

/// The capabilities a node advertises about itself.
#[derive(Default)]
struct OwnCaps {
    caps: RwLock<HashMap<CapKey, Box<dyn Any + Send + Sync>>>,
}

impl OwnCaps {
    fn can<C: Capability>(&self, cap: C) {
        self.caps
            .write()
            .insert(CapKey(C::NAME.to_string()), Box::new(cap));
    }

    fn support<C: Capability + Clone>(&self) -> Option<C> {
        self.caps
            .read()
            .get(C::NAME)
            .and_then(|cap| cap.downcast_ref::<C>())
            .cloned()
    }
}

impl fmt::Debug for OwnCaps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `dyn Any` is not `Debug`; show which capabilities are present, not their contents.
        f.debug_set().entries(self.caps.read().keys()).finish()
    }
}
