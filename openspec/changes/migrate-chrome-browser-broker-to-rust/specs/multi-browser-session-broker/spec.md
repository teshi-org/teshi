## ADDED Requirements

### Requirement: Project-scoped requests on a user-scoped broker
The broker process and extension session registry SHALL be shared per operating-system user, while every agent operation SHALL carry a canonical project context and caller identity. Leases and privileged grants SHALL be bound to their owning project, caller, broker generation, and browser Profile; extension Profiles SHALL NOT be owned by the project that first starts the broker.

#### Scenario: A second project reuses the running broker
- **WHEN** Project B starts Chrome automation after Project A has started the user broker
- **THEN** Project B SHALL receive its own endpoint pointer and SHALL be able to use a registered Profile under Project B's own lease and policy context

#### Scenario: One project closes while another owns a lease
- **WHEN** Project A exits or its endpoint is removed while Project B holds a valid Profile lease
- **THEN** the shared broker SHALL remain running and SHALL preserve Project B's lease and pending work

#### Scenario: A lease token is used from another project
- **WHEN** a caller supplies a valid lease token with a project or caller identity different from its owner
- **THEN** the broker SHALL reject the operation before dispatch and SHALL not expose the target page data

#### Scenario: Extension heartbeats use a legacy project field
- **WHEN** a compatible extension heartbeat includes `project_root`
- **THEN** the broker SHALL treat it only as legacy metadata and SHALL NOT bind the Profile, heartbeat, or preview stream to the first starter's project
