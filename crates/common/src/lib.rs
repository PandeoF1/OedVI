//! Types and algorithms shared by the server, the client library and the launcher.

pub mod builder;
pub mod hash;
pub mod manifest;
pub mod paths;
pub mod stats;
pub mod store;

pub use builder::{BuilderDef, BuilderList, BuilderLoadError};
pub use manifest::{FileEntry, GroupInfo, Manifest, ScanStatus};

/// Version of the HTTP API. Bumped on incompatible changes.
pub const API_VERSION: u32 = 1;
