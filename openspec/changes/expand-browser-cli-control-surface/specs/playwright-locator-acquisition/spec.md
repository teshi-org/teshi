## MODIFIED Requirements

### Requirement: Locator acquisition does not invent test behavior
Teshi SHALL limit locator acquisition to inspection, target disambiguation, locator rendering, and verification and SHALL NOT invent navigation, input values, assertions, or destructive actions not requested by the caller. Executing a returned candidate SHALL require a separate control operation, valid lease, matching target and revision, and explicit action.

#### Scenario: Agent asks for a submit-button locator
- **WHEN** Teshi resolves and verifies the requested element
- **THEN** it SHALL return locator candidates without clicking the button unless the caller separately submits an authorized execution operation

#### Scenario: Agent separately executes the candidate
- **WHEN** the caller later supplies the candidate, matching page revision, explicit action, target, and lease
- **THEN** Teshi SHALL treat execution as a new correlated request and SHALL re-verify before mutation

## ADDED Requirements

### Requirement: Executable candidate fidelity
A structured locator candidate SHALL contain or reference all semantic, frame, shadow, and revision information required for the shared execution layer to re-resolve it without CSS-only conversion.

#### Scenario: Candidate targets an element inside a frame
- **WHEN** a verified candidate contains frame context and is passed to execution
- **THEN** Teshi SHALL retain the frame context during re-verification and action dispatch
