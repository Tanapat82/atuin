//! Capability system used by atuin.
//!
//! # Context
//!
//! A node advertises capabilities about itself and, if it is a client, can read the server's.
//!
//! Atuin's client and server versions are not necessarily always compatible. There are features
//! that clients may support, but outdated servers will not.
//!
//! The capability system is designed to help us bridge the gap between the two.
//!
//! # Design
//!
//! - Each capability has a unique `CRI` (capability resource identifier), eg.
//!   `sh.atuin.server/capabilities`.
//! - Each capability has arbitrary associated data, for example `{ "version": 1 }`.
//!
//! The client passes a header with each request it makes, `x-atuin-capabilities-known: <hash>`
//! which communicates to the server what capabilities the client is aware of. If the server's
//! capability hash does not match that of what the client believes, the server rejects the request
//! with a 412, after which the client polls `/api/v0/capabilities` to get the new capability list
//! as well as the new hash of the capability list.
//!
//! The client then passes this updated hash back to the server and all is well.
//!
//! The server capabilities are sent with every response as part of `x-atuin-capabilities-available`
//! in order to eagerly communicate to the client that the capability set needs to be updated
//! (hopefully to avoid unnecessary 412s).
//!
//! # Implementation
//!
//! The client side is implemented as reqwest middleware in [`client::CapClient`].
//! The server side is implemented as a plain struct that can be embedded in any server, in
//! [`client::CapServer`].

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

/// The capability-negotiation protocol itself, expressed as a capability.
///
/// A server that speaks capabilities advertises this, so a client can observe -- from the
/// capability set alone -- that the protocol is supported, and at which version. It is
/// deliberately self-referential: receiving any capability document already implies the server
/// understands capabilities. Naming that fact gives the negotiation machinery a concrete
/// capability to carry today, while the richer feature-specific ones live with their features.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitiesCap {
    /// The version of the capability-negotiation protocol the server implements.
    pub version: u32,
}

impl Capability for CapabilitiesCap {
    const NAME: &'static str = "sh.atuin.server/capabilities";
}

/// The capabilities a server advertises, as returned from its capabilities endpoint.
#[derive(Debug, Serialize, Deserialize)]
pub struct CapabilitiesResponse {
    /// An opaque capability token issued by the server.
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
