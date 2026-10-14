## Why

Teshi supports browser and Windows application tests but cannot drive arbitrary terminal applications through its supported test workflow. Its own flagship TUI consequently lacks executable interface regressions.

## What Changes

- Add a Rust TUI runner that launches applications in a real pseudo-terminal, sends input, resizes the terminal, and asserts the rendered screen.
- Expose TUI execution through the Teshi CLI and existing scenario execution/reporting workflow, including failure evidence and explicit errors for unsupported steps.
- Add executable BDD regressions for Teshi's own TUI using isolated project fixtures.
- Evaluate existing Rust terminal testing frameworks before choosing the execution backend.

## Capabilities

### New Capabilities

- `tui-testing`: Terminal application automation, scenario execution, lifecycle management, evidence, and Teshi self-tests.

### Modified Capabilities

None.

## Impact

Rust workspace, CLI command routing, scenario runner selection and event reporting, test fixtures, CI and development documentation. Existing browser and WinApp contracts remain compatible. Terminal testing requires a PTY backend on each supported host.
