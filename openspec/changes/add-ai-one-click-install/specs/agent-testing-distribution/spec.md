## ADDED Requirements

### Requirement: AI-guided install runbook
Teshi SHALL publish an English root `AI_INSTALL.md` that a coding agent can follow to install the teshi CLI, guide the user to load the `teshi-bridge` Chrome extension, and install bundled Agent Skills. The repository README SHALL include a one-sentence English pointer to that runbook.

#### Scenario: Agent follows the README pointer
- **WHEN** a coding agent is asked to install teshi for browser locator use
- **THEN** it SHALL be able to open `AI_INSTALL.md` from the README pointer and follow it without downloading Skill files from GitHub

#### Scenario: Chinese README points at the same runbook
- **WHEN** a reader opens `README_zh.md`
- **THEN** it SHALL include a pointer to the same English `AI_INSTALL.md`

### Requirement: Local Skill install command
Teshi SHALL provide `teshi install-skill` that copies Skills from the local install or source tree into `~/.agents/skills/<name>` and, when the parent directory already exists, creates a symlink at each supported Agent discovery path.

#### Scenario: Dry-run writes nothing
- **WHEN** the user runs `teshi install-skill --dry-run`
- **THEN** the command SHALL print the planned copies and links and SHALL NOT create, replace, or delete any files

#### Scenario: Non-TTY requires explicit confirmation
- **WHEN** stdin is not a TTY and `--yes` is absent
- **THEN** the command SHALL refuse to write files

#### Scenario: Packaged Skill is installed from share
- **WHEN** the teshi executable's adjacent or parent `share/teshi-browser-testing/skills` directory contains `playwright-locator`
- **THEN** `teshi install-skill` SHALL copy that Skill into `~/.agents/skills/playwright-locator`

#### Scenario: Development tree resolves the packaged Skill
- **WHEN** the executable lives under a teshi checkout that contains `agent-packages/teshi-browser-testing/skills`
- **THEN** `teshi install-skill` SHALL use that packaged Skill directory rather than fetching files from the network

#### Scenario: Missing source fails with install guidance
- **WHEN** neither the share layout nor a source checkout Skill tree can be resolved
- **THEN** the command SHALL fail with a message that points at winget or GitHub Release installation and SHALL NOT instruct the agent to download `SKILL.md` from GitHub

#### Scenario: Discovery parent absent is skipped
- **WHEN** a supported Agent skills parent directory does not exist
- **THEN** the command SHALL skip creating that symlink and SHALL still install the canonical `~/.agents/skills/<name>` copy

#### Scenario: Real directory at a discovery path is preserved
- **WHEN** a supported Agent discovery path already contains a non-symlink directory of the same Skill name
- **THEN** the command SHALL skip that path even if `--yes` is set and SHALL report that it did not overwrite the directory

#### Scenario: Packaged Skill wins on name collision
- **WHEN** both the packaged tree and a repository `skills/` directory contain a Skill with the same folder name
- **THEN** the command SHALL install the packaged copy
