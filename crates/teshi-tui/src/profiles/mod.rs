//! TUI helpers over the shared `teshi-engine` model-profile store.
//!
//! Persistence lives under app data `model-profiles/` (same as Desktop/daemon).

mod helpers;

pub use helpers::{
    delete_profile, load_all, normalize_provider, profile_temperature, read_active_id, save,
    set_profile_temperature, write_active_id,
};
pub use teshi_engine::ModelProfile;
