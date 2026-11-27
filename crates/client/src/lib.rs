//! Everything the launcher does, without the UI: talking to the server,
//! keeping the local game files in sync, and fetching, building and running
//! builders. The CLI (`oedvi`) and the Tauri launcher are thin layers on top.

pub mod api;
pub mod backup;
pub mod builder;
pub mod git;
pub mod local;
pub mod metrics;
pub mod monitor;
pub mod prereq;
pub mod process;
pub mod selection;
pub mod settings;
pub mod state;
pub mod sync;

pub use api::Api;
pub use settings::{Layout, Settings};
