## Context

The project's `src/agent/skills/` module provides a custom `.tskill` template system:
- `SkillDefinition` / `SkillRegistry` types with keyword-based matching
- A `.tskill` file parser (YAML frontmatter + Markdown body)
- Used by `app.rs` to inject a template catalog into the AI system prompt
- Used by `validator.rs` to match feature names against skill templates for coverage checking

Separately, the agent definition system has a `skills` config field that resolves skill directory paths during agent initialization. These paths point to standard skill directories.

The `.tskill` system is dead code — no `.tskill` files exist. It should be removed, but the agent's skill directory resolution should be preserved.

## Goals / Non-Goals

**Goals:**
- Remove all `.tskill`-specific code (mod.rs, loader.rs, app.rs consumer, validator.rs consumer)
- Preserve the `skills` field on `AgentDefinition` and its YAML config (`SkillsConfigRaw`)
- Preserve `resolve_skills()` in the loader so agent initialization still resolves skill dirs
- Ensure the project compiles and all existing tests pass

**Non-Goals:**
- Changing the standard skill system
- Changing how skill directories are resolved in the loader (only removing the consumer)
- Modifying the BDD/gherkin validation logic beyond removing the skill-matching part

## Decisions

| Decision | Rationale |
|---|---|
| **Delete** `src/agent/skills/` module entirely | The whole module is `.tskill`-specific; nothing to salvage |
| **Keep** `skills: Vec<PathBuf>` in `AgentDefinition` | Agent initialization needs to carry skill directory paths |
| **Keep** `SkillsConfigRaw` / `skills` field in `AgentDefinitionRaw` | YAML config surface is the mechanism for specifying skill dirs |
| **Keep** `resolve_skills()` in loader | It resolves skill dir paths during agent init — that logic is still needed |
| **Remove** `skill_registry` from `App` | Only existed to load `.tskill` files and inject template catalog |
| **Remove** `load_skill_registry()` / `reload_skills_for_profile()` from `App` | Only consumed `.tskill` files |
| **Remove** skill catalog section from `ai_system_prompt()` | The "Available Generation Templates" with `load_skill` tool mention was `.tskill`-specific |
| **Remove** `SkillRegistry` param from `check_coverage()` in validator | The skill-matching logic was `.tskill`-specific; remove the param and early-return the function |
| **Remove** `#[expect(dead_code)]` only when the annotated item is removed | Don't touch unrelated dead_code annotations |

## Risks / Trade-offs

- [Low] **`check_coverage` loses skill-based suggestions** — This function used `.tskill` templates to suggest missing scenario types. After removal it becomes a no-op (always returns empty). If this feature is needed in the future, it should use the standard skill system instead.
- [Low] **Agent YAMLs with `skills.dirs` become metadata-only** — The directories are still resolved and stored on `AgentDefinition`, but no code consumes them at runtime. This is acceptable — skill loading is handled separately by the skill system.
