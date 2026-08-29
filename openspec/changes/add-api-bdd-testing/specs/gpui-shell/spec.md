## ADDED Requirements

### Requirement: Shared GPUI Run/API inspect surface

The shared `teshi-ui` root shell SHALL expose a Run/API inspect surface in addition to Browser, WinApp, and Settings. The default landing surface SHALL remain the existing main/WinApp preview behavior. The Run/API surface SHALL be implemented in `teshi-ui` without depending on `teshi-engine` or `teshi-agent`; platform I/O SHALL go through a backend trait supplied by desktop and web hosts.

#### Scenario: User opens Run from the shell header

- **WHEN** the user activates the Run/API navigation control
- **THEN** the shell SHALL show the Run/API inspect surface and MUST NOT replace it with the LLM settings form

#### Scenario: Desktop and web share the view

- **WHEN** `teshi-desktop` and `teshi-web` are built
- **THEN** both SHALL render the same `teshi-ui` Run/API view type with host-specific backends
