## Purpose

Defines how Teshi's browser locator workflow, extension assets, and agent integration metadata are packaged, installed, discovered, and versioned outside the source repository.

## ADDED Requirements

### Requirement: Installable browser locator workflow package
Teshi SHALL publish an installable package containing a focused Playwright locator Skill, local MCP metadata, browser-extension installation resources, and the documentation referenced by the workflow.

#### Scenario: Agent package is installed
- **WHEN** a user installs the Teshi testing package in a supported Codex or compatible agent environment
- **THEN** the browser locator Skill SHALL be discoverable without copying files from the Teshi source checkout

### Requirement: Repository-local Skill compatibility
The distributed browser locator workflow SHALL remain usable from a repository-local `.agents/skills` location using the Open Agent Skills directory format.

#### Scenario: Team vendors the locator Skill
- **WHEN** a consumer repository places the distributed Skill beneath `.agents/skills`
- **THEN** a compatible agent launched in that repository SHALL discover it by its declared name and description

### Requirement: Declared runtime dependencies
The package SHALL declare compatible Teshi CLI, broker protocol, browser extension, operating-system, Chromium, and optional MCP versions required by the workflow.

#### Scenario: Extension is unavailable
- **WHEN** an agent activates the locator Skill without a compatible connected extension
- **THEN** the workflow SHALL stop before debugger attachment or browser mutation and report installation or version guidance

### Requirement: Release artifact completeness
Teshi installers and portable archives that advertise browser locator support SHALL include or link unambiguously to the compatible extension bundle, Skills, plugin metadata, MCP configuration, and referenced documentation.

#### Scenario: Installed workflow resolves resources
- **WHEN** the Skill is loaded from an installed release
- **THEN** every bundled reference and required local runtime resource SHALL resolve without access to the Teshi source repository

### Requirement: Versioned compatibility contract
The package SHALL declare its compatible CLI, broker protocol, and extension ranges and SHALL update them when a referenced operation or schema changes incompatibly.

#### Scenario: Package and extension are incompatible
- **WHEN** the workflow detects an extension outside its compatible range
- **THEN** it SHALL fail preflight with detected and required versions rather than attempting locator acquisition

### Requirement: Packaged multi-profile guidance
The distribution SHALL document how to install the extension into dedicated browser profiles, assign display labels, discover sessions, and allocate different profiles to concurrent agents.

#### Scenario: User prepares two agent profiles
- **WHEN** the user follows the packaged multi-profile setup guidance
- **THEN** both extension instances SHALL be discoverable as distinct Teshi sessions without configuring separate broker ports
