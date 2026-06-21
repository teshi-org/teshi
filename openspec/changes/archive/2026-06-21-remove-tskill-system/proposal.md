## Why

The project includes a built-in "Skill/Template" system (`.tskill` files) for AI generation guidance — but this feature is unused (no `.tskill` files exist). The proper mechanism for skill management uses standard skill directories rather than a custom template format. Removing the custom `.tskill` infrastructure eliminates dead code while keeping the agent's ability to reference skill directories.

## What Changes

- **Remove** the entire `src/agent/skills/` module (`SkillRegistry`, `SkillDefinition`, `.tskill` file parser)
- **Keep** the `skills` field in `AgentDefinition` (agent initialization still needs skill directory paths for integration with the standard skill system)
- **Keep** the `SkillsConfigRaw` YAML type and `resolve_skills()` in the loader — skill directory resolution is part of agent initialization
- **Remove** the `skill_registry` field and all `.tskill`-loading code from `src/app.rs` (`load_skill_registry`, `reload_skills_for_profile`)
- **Remove** the skill template catalog injection from `ai_system_prompt()`
- **Remove** the `SkillRegistry` dependency from `agent/validator.rs` (`check_coverage` skill matching)
- No breaking changes — agent YAMLs with `skills.dirs` continue to parse, paths are resolved, but the `.tskill` consumer is gone

## Capabilities

### New Capabilities

*(None — this is a removal)*

### Modified Capabilities

*(None — no spec-level requirement changes)*

## Impact

- **Code removed:** ~350 lines across 5 source files
- **Preserved:** `skills: Vec<PathBuf>` field on `AgentDefinition` and its YAML config — agent initialization continues to resolve skill directory paths
- **Dependencies:** No external dependency changes
