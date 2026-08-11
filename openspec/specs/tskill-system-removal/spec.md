# TSkill System Removal

## Purpose

Records the removal of the custom `.tskill` template system. No new capabilities
were added — this is a pure cleanup of dead code.

## Requirements

### Requirement: Legacy TSkill runtime remains removed

Teshi SHALL NOT parse or load `.tskill` templates, construct a `SkillRegistry` or
`SkillDefinition`, inject registry content into application system prompts, or run
the former skill-based coverage validation.

#### Scenario: Project contains a legacy tskill file

- **WHEN** Teshi loads a project containing a `.tskill` file
- **THEN** the application SHALL NOT load that file into a legacy TSkill registry

#### Scenario: Feature coverage is validated

- **WHEN** Teshi validates feature coverage
- **THEN** it SHALL NOT invoke the removed skill-based coverage checker

### Requirement: Generic agent skill paths remain supported

Agent definitions SHALL retain their generic skill-path configuration, raw YAML
skill configuration, and loader path resolution independently of the removed
`.tskill` runtime.

#### Scenario: Agent definition declares skill paths

- **WHEN** the loader reads an agent definition with configured skill paths
- **THEN** it SHALL resolve those paths and retain them on the loaded agent definition
