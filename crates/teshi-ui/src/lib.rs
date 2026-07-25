//! Shared GPUI views for teshi desktop and web shells.
//!
//! This crate must not depend on `teshi-engine` or `teshi-agent`. Platform I/O
//! goes through [`LlmConfigBackend`].

mod app_shell;
mod backend;
mod llm_config_view;

pub use app_shell::{AppShell, ShellSurface};
pub use backend::{
    ApiStyleDto, LlmConfigBackend, LlmConfigSnapshot, LlmConfigUpdate, ModelProfileListSnapshot,
    ModelProfileSnapshot, ModelProfileUpdate, SharedLlmBackend,
};
pub use llm_config_view::{LlmConfigView, bind_llm_config_keys};
