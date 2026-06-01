//! Controller — implements `MetaSwarmService`.

pub mod inflight;
pub mod iterate;
pub mod reload;
mod service;
mod state;

pub use inflight::InflightSemaphore;
pub use reload::{
    apply_reload_from_disk, apply_reload_with_manifest, build_inline_manifest, ReloadOutcome,
};
pub use service::ControllerService;
pub use state::ControllerState;
