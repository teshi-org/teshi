# Remove TSkill System — Specs

No new capabilities or modified capability requirements. This is a pure removal
of the custom `.tskill` template infrastructure while preserving the agent's
skill directory configuration (`skills` field on `AgentDefinition`).

Preserved:
- `skills: Vec<PathBuf>` field on `AgentDefinition`
- `SkillsConfigRaw` YAML config type
- `resolve_skills()` path resolution in loader

Removed:
- `SkillRegistry` / `SkillDefinition` / `.tskill` parser
- App-level skill registry loading and system prompt injection
- Validator's skill-based coverage checking
