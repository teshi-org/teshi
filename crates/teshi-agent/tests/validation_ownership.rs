//! Architecture guard: agent and Skills consume the core validator instead of
//! carrying a second dialect keyword rule table.

use std::fs;
use std::path::Path;

#[test]
fn agent_validator_is_a_core_adapter() {
    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("validator.rs"),
    )
    .expect("read agent validator source");

    assert!(source.contains("validate_feature_source"));
    assert!(source.contains("validate_core_project"));
    assert!(!source.contains("GherkinLanguages"));
    assert!(!source.contains("match_step_prefix"));
    assert!(!source.contains("const STEP_KEYWORDS"));
    assert!(!source.contains("static STEP_KEYWORDS"));
}

#[test]
fn packaged_skills_delegate_syntax_validation_to_teshi() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");

    for skill in ["teshi", "bdd-feature"] {
        let source = fs::read_to_string(repo_root.join("skills").join(skill).join("SKILL.md"))
            .expect("read packaged Skill");
        assert!(
            source.contains("teshi check") || source.contains("$TESHI check"),
            "{skill} Skill must delegate validation to `teshi check`"
        );
        assert!(
            !source.contains("STEP_KEYWORDS") && !source.contains("GherkinLanguages"),
            "{skill} Skill must not define a dialect keyword table"
        );
    }
}

#[test]
fn repository_has_one_gherkin_validation_owner() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");

    let core_validation = fs::read_to_string(repo_root.join("crates/teshi-core/src/validation.rs"))
        .expect("read Core validation implementation");
    assert!(core_validation.contains("pub fn validate_feature_source"));
    assert!(core_validation.contains("GherkinLanguages"));

    for duplicate in [
        "src/gherkin.rs",
        "src/gherkin_lang.rs",
        "src/validation.rs",
        "src/highlight.rs",
    ] {
        assert!(
            !repo_root.join(duplicate).exists(),
            "repository root must not carry a duplicate Gherkin implementation: {duplicate}"
        );
    }

    let engine_gherkin = fs::read_to_string(repo_root.join("crates/teshi-engine/src/gherkin.rs"))
        .expect("read engine Gherkin adapter");
    assert!(engine_gherkin.contains("teshi_core::"));
    for duplicate in ["STEP_KEYWORDS", "GherkinLanguages", "match_step_prefix"] {
        assert!(
            !engine_gherkin.contains(duplicate),
            "engine must not duplicate Core Gherkin ownership: {duplicate}"
        );
    }
}
