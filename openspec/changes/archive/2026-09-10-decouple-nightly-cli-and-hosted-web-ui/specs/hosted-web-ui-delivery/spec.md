## ADDED Requirements

### Requirement: Hugo Pages publishes the complete GPUI WASM application
The `teshi-org.github.io` Pages workflow SHALL build the complete GPUI WASM Web UI from a resolved immutable Teshi source SHA and SHALL publish its loader, JavaScript glue, WASM, static resources, and compatibility manifest under `https://teshi.org/app/` together with the Hugo site.

#### Scenario: Pages build succeeds
- **WHEN** the Hugo Pages workflow builds an approved Teshi source revision
- **THEN** the uploaded Pages artifact SHALL contain a runnable `/app/` GPUI WASM application and `/app/ui-manifest.json`
- **AND** the manifest SHALL record the full Teshi source SHA used for the build

#### Scenario: Web UI build or validation fails
- **WHEN** GPUI WASM compilation, binding generation, manifest validation, or Web UI smoke validation fails
- **THEN** the Pages workflow SHALL fail before deployment and SHALL leave the currently deployed site unchanged

### Requirement: Hosted UI deployment is latest-only
The Pages deployment SHALL expose only the current Web UI at the canonical `/app/` route and SHALL NOT publish a historical UI catalog or select a UI version based on a local CLI version.

#### Scenario: A new UI is deployed
- **WHEN** a new Pages artifact becomes active
- **THEN** `/app/` SHALL load that deployment's current entrypoint and content-addressed assets
- **AND** no historical compatibility fallback route SHALL be generated

### Requirement: Nightly CLI publication excludes the complete Web UI
The nightly CLI workflow SHALL NOT install Web-only build tooling, compile `apps/teshi-web`, upload a Web distribution artifact, stage `share/web`, or inventory complete Web UI files in nightly installer/update manifests.

#### Scenario: Ordinary nightly CLI change is published
- **WHEN** `nightly.yml` invokes the reusable release workflow in its nightly Windows-installer mode
- **THEN** no GPUI WASM target installation, `wasm-bindgen` installation, and the Web UI build SHALL be skipped
- **AND** the resulting nightly installer SHALL contain no complete GPUI WASM Web UI payload

#### Scenario: Shared release workflow is inspected
- **WHEN** Web packaging conditions are evaluated for nightly and non-nightly modes
- **THEN** the conditions SHALL prove that nightly omits the Web UI without silently changing separately configured stable packaging behavior
