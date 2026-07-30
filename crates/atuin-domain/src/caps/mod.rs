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
//! `client::CapServer`.
//!
//! # TODO
//!
//! The eager `x-atuin-capabilities-available` path described above is not implemented yet: the
//! server only sends that header on a 412, so a stale client currently learns of a capability
//! change on its next rejected request rather than preemptively from an earlier response.

use parking_lot::RwLock;
use std::{any::Any, borrow::Borrow, collections::BTreeMap, fmt};

use serde::{Serialize, de::DeserializeOwned};

pub mod http;

mod all;
mod client;
mod middleware;
mod server;

pub use all::CapabilitiesCap;
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

/// A dyn-compatible version of [`Capability`].
pub trait DynCapability: Any + Send + Sync {
    /// Get the name of this capability.
    fn name(&self) -> &'static str;

    /// Convert this capability into a JSON value.
    fn json(&self) -> Result<serde_json::Value, serde_json::Error>;
}

impl<C: Capability> DynCapability for C {
    fn name(&self) -> &'static str {
        C::NAME
    }

    fn json(&self) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::to_value(self)
    }
}

/// The capabilities a node advertises about itself.
#[derive(Default)]
struct CapsBundle {
    caps: RwLock<BTreeMap<CapKey, Box<dyn DynCapability>>>,
}

impl CapsBundle {
    /// Register a capability this node advertises.
    fn add<C: Capability>(&self, cap: C) {
        self.caps
            .write()
            .insert(CapKey(C::NAME.to_string()), Box::new(cap));
    }

    /// Check whether this node advertises the given capability.
    fn get<C: Capability + Clone>(&self) -> Option<C> {
        self.caps
            .read()
            .get(C::NAME)
            .and_then(|cap| {
                let cap: &dyn Any = &**cap;
                cap.downcast_ref::<C>()
            })
            .cloned()
    }
}

impl fmt::Debug for CapsBundle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `dyn Any` is not `Debug`; show which capabilities are present, not their contents.
        f.debug_set().entries(self.caps.read().keys()).finish()
    }
}
