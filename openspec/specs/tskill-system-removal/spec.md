# TSkill System Removal

## Purpose

Records the removal of the custom `.tskill` template system. No new capabilities
were added — this is a pure cleanup of dead code.

## Removed

- `SkillRegistry` / `SkillDefinition` / `.tskill` parser
- App-level skill registry loading and system prompt injection
- Validator's skill-based coverage checking

## Preserved

- `skills: Vec<PathBuf>` field on `AgentDefinition`
- `SkillsConfigRaw` YAML config type
- `resolve_skills()` path resolution in loader
