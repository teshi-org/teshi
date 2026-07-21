//! Daemon manifest DTO (pure data shape).
//! I/O and lifecycle logic lives in `teshi-engine`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Written by the daemon; read by all clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonManifest {
    pub pid: u32,
    pub port: u16,
    pub started: DateTime<Utc>,
}
