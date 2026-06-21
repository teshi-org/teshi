## 1. Remove `src/agent/skills/` Module

- [x] 1.1 Delete `src/agent/skills/mod.rs` (SkillRegistry, SkillDefinition, matching logic)
- [x] 1.2 Delete `src/agent/skills/loader.rs` (.tskill file parser)
- [x] 1.3 Remove `pub mod skills;` declaration from `src/agent/mod.rs`

## 2. Clean Up `src/agent/validator.rs`

- [x] 2.1 Remove the `SkillRegistry` import
- [x] 2.2 Remove the `skill_registry: &SkillRegistry` parameter from `check_coverage()`
- [x] 2.3 Remove the skill-matching logic inside `check_coverage()` (lines that reference `skill_registry`)
- [x] 2.4 Update all call sites of `check_coverage()` to drop the skill_registry argument
- [x] 2.5 Remove or update the tests that depend on SkillRegistry

## 3. Clean Up `src/agent/definition.rs`

- [x] 3.1 Keep `skills: Vec<PathBuf>` in `AgentDefinition` — no change needed
- [x] 3.2 Keep `SkillsConfigRaw` struct — no change needed
- [x] 3.3 Keep `skills: Option<SkillsConfigRaw>` in `AgentDefinitionRaw` — no change needed

## 4. Clean Up `src/agent/loader.rs`

- [x] 4.1 Keep `resolve_skills()` method — no change needed
- [x] 4.2 Remove the now-dead `project_dir` field from `Resolver` if no longer used elsewhere
- [x] 4.3 Clean up unused imports after module removal

## 5. Clean Up `src/app.rs`

- [x] 5.1 Remove the `skill_registry` field from the `App` struct
- [x] 5.2 Remove the `load_skill_registry()` function
- [x] 5.3 Remove the `reload_skills_for_profile()` method
- [x] 5.4 Remove skill catalog injection from `ai_system_prompt()` (the "Available Generation Templates" block)
- [x] 5.5 Remove `skill_registry` initialization from all `App` constructors
- [x] 5.6 Remove the `self.reload_skills_for_profile()` call when switching agents
- [x] 5.7 Clean up any unused imports

## 6. Verify

- [x] 6.1 Run `cargo check` to ensure the project compiles
- [x] 6.2 Run `cargo test` to ensure all tests pass
- [x] 6.3 Run `cargo clippy` to catch any leftover dead code issues
