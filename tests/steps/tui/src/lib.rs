use std::path::Path;

/// Returns true if the scenario name indicates a TUI e2e test.
pub fn is_tui_scenario(scenario: &str) -> bool {
    scenario.contains("TUI") || scenario.contains("tui")
}

/// Returns true if the current host supports TUI e2e tests (Linux only).
pub fn tui_e2e_host_supported() -> bool {
    cfg!(target_os = "linux")
}

/// Returns true if the scenario is implemented and can be run.
pub fn supports_scenario(scenario: &str) -> bool {
    // All non-TUI scenarios are supported; TUI scenarios need Linux
    if is_tui_scenario(scenario) {
        return tui_e2e_host_supported();
    }
    true
}

/// Run a single BDD scenario using the given teshi binary.
pub fn run_scenario(scenario: &str, teshi_bin: &Path) -> anyhow::Result<()> {
    let _ = (scenario, teshi_bin);
    anyhow::bail!("TUI e2e steps not yet implemented")
}
