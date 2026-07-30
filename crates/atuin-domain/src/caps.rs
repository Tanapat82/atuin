//! Capability types negotiated between Atuin nodes.
//!
//! The [`Capability`] trait and the negotiation machinery live in `atuin-common`; this module
//! holds the concrete capability *types* that are part of the domain.

use atuin_common::caps::Capability;
use serde::{Deserialize, Serialize};

/// The server accepts history packfiles. Advertised by the server; its presence gates client-side
/// packfile upload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackfileCap {
    /// The capability version the server speaks.
    pub version: u32,
}

impl Capability for PackfileCap {
    const NAME: &'static str = "sh.atuin.server/records.bundle";
}
