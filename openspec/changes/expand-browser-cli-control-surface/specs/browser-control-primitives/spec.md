## ADDED Requirements

### Requirement: CLI-owned Chrome broker bootstrap
Teshi browser CLI operations that require Chrome SHALL discover and reuse a compatible user-session loopback broker and SHALL start it idempotently when it is absent, without requiring Teshi Desktop to be open.

#### Scenario: First CLI command starts the broker
- **WHEN** a user invokes Chrome session discovery while no broker is running
- **THEN** Teshi SHALL start one broker under a startup lock, wait for readiness, and perform discovery

#### Scenario: Incompatible broker is already running
- **WHEN** a CLI command discovers a broker with an incompatible protocol
- **THEN** Teshi SHALL fail with detected and required versions and SHALL NOT terminate or replace the broker implicitly

### Requirement: Request-scoped project context
A shared user-session broker SHALL canonicalize each operation's project root and SHALL use that request context for policy, grants, uploads, managed artifacts, and cleanup rather than the project that started the broker.

#### Scenario: Two projects reuse one broker
- **WHEN** a caller from a second project creates a grant or cleans managed artifacts
- **THEN** policy evaluation and filesystem effects SHALL remain within the second caller's canonical project root

### Requirement: Revision-bound element references
Teshi SHALL issue compact snapshot element references bound to the complete browser target, snapshot identity, page-context revision, and frame or shadow context.

#### Scenario: Caller uses a current reference
- **WHEN** a leased caller executes an action using a reference from the current target and revision
- **THEN** Teshi SHALL resolve the reference only within that recorded context

#### Scenario: Caller uses a stale reference
- **WHEN** navigation, document replacement, target mismatch, expiry, or eviction invalidates a reference
- **THEN** Teshi SHALL return `stale_element_reference` and SHALL perform no mutation

### Requirement: First-class structured locator execution
Teshi SHALL execute a separately authorized action against a verified structured locator candidate without converting semantic, frame, or shadow context to a CSS-only selector.

#### Scenario: Agent executes a verified role locator
- **WHEN** an agent supplies a verified role/name candidate, matching page revision, complete target, and valid lease
- **THEN** Teshi SHALL re-verify the candidate and execute the requested action against the same resolved element

#### Scenario: Candidate is no longer valid
- **WHEN** candidate re-verification detects a stale revision, non-unique match, or different target
- **THEN** Teshi SHALL fail closed and SHALL NOT execute the action

### Requirement: Distinct DOM and pointer activation
Teshi SHALL expose DOM activation and CDP-backed pointer activation as distinct actions with explicit result metadata.

#### Scenario: Caller requests pointer activation
- **WHEN** a visible actionable element is targeted with the pointer action
- **THEN** Teshi SHALL scroll it into view, calculate a verified hit point, dispatch a bounded pointer sequence, and report the coordinates and focus effect without exposing unrelated page data

### Requirement: Consistent browser action contract
Chrome extension and embedded browser modes SHALL advertise and validate their supported click, fill, type, key, select, assertion, navigation, and upload actions through the shared typed operation contract.

#### Scenario: Backend does not support an action
- **WHEN** a caller requests an action not advertised by the selected backend
- **THEN** Teshi SHALL return `unsupported_browser_action` before mutation with supported alternatives

### Requirement: Typed post-action waits
Browser actions SHALL optionally include bounded typed wait conditions and SHALL report action and wait outcomes separately.

#### Scenario: Click succeeds and condition becomes true
- **WHEN** a click succeeds and its requested URL, text, element-state, revision, or load condition becomes true before timeout
- **THEN** Teshi SHALL return both action success and wait success with observed non-sensitive evidence

#### Scenario: Click succeeds but wait times out
- **WHEN** the action completes but its post-condition does not become true before timeout
- **THEN** Teshi SHALL report the completed action and a distinct `browser_wait_timeout` without retrying the mutation

### Requirement: Explicit tab and window lifecycle
Teshi SHALL list and look up browser/profile/tab identities and SHALL support lease-scoped tab activation, creation, closure, window creation, and optional tab grouping.

#### Scenario: Agent opens a tab in a selected profile
- **WHEN** an agent with a valid profile lease requests a new tab or window for an explicit URL
- **THEN** Teshi SHALL return the new complete target identity and SHALL NOT reroute subsequent commands implicitly

#### Scenario: Tab identifier is ambiguous
- **WHEN** a tab identifier matches more than one extension instance and the caller omits the session
- **THEN** Teshi SHALL return `ambiguous_browser_target` and SHALL perform no tab operation

### Requirement: Unique profile label management
Teshi SHALL allow CLI callers to set, clear, and resolve display labels while retaining opaque extension identity as the routing key.

#### Scenario: Label collides
- **WHEN** a requested label is already used by another live profile
- **THEN** Teshi SHALL reject the label assignment with the conflicting opaque identities and SHALL NOT create ambiguous routing
