//! Model profile management — storage, loading, and active-profile tracking.
//!
//! Profiles are stored as individual TOML files under
//! `~/.config/teshi/models/{id}.toml`. The active profile ID is persisted
//! in `~/.config/teshi/model_profile`.

pub mod schema;

pub use schema::ModelProfile;
