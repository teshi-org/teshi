//! Shared GPUI views for teshi desktop and web shells.
//!
//! This crate must not depend on `teshi-engine` or `teshi-agent`. Platform I/O
//! goes through [`LlmConfigBackend`] and [`BrowserSessionsBackend`].

mod app_shell;
mod backend;
mod browser_sessions_view;
mod llm_config_view;
mod run_view;
mod winapp_preview;

pub use app_shell::{AppShell, ShellSurface};
pub use backend::{
    ApiRunBackend, ApiRunEventDto, ApiScenarioSnapshot, ApiStyleDto, BrowserLeaseSnapshot,
    BrowserMetadataSnapshot, BrowserSessionIdentitySnapshot, BrowserSessionListSnapshot,
    BrowserSessionSnapshot, BrowserSessionsBackend, BrowserTabSnapshot, BrowserTabTarget,
    BrowserWindowSnapshot, LlmConfigBackend, LlmConfigSnapshot, LlmConfigUpdate,
    ModelProfileListSnapshot, ModelProfileSnapshot, ModelProfileUpdate, SharedApiRunBackend,
    SharedBrowserSessionsBackend, SharedLlmBackend,
};
pub use browser_sessions_view::BrowserSessionsView;
pub use llm_config_view::{LlmConfigView, bind_llm_config_keys};
pub use run_view::ApiRunView;
pub use winapp_preview::{PreviewStatus, WinAppPreview};
