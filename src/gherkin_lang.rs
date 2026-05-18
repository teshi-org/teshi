//! Gherkin multi-language keyword support.
//!
//! Loads the Cucumber Gherkin languages JSON and provides
//! per-language keyword lookup and normalisation.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

/// Normalised step keyword type, independent of language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepKeywordType {
    Given,
    When,
    Then,
    And,
    But,
}

/// Structural Gherkin keyword type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuralType {
    Feature,
    Background,
    Scenario,
    ScenarioOutline,
    Examples,
    Rule,
}

// ── Raw deserialisation ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct RawLanguage {
    name: String,
    native: String,
    feature: Vec<String>,
    background: Vec<String>,
    scenario: Vec<String>,
    #[serde(rename = "scenarioOutline")]
    scenario_outline: Vec<String>,
    examples: Vec<String>,
    given: Vec<String>,
    when: Vec<String>,
    then: Vec<String>,
    and: Vec<String>,
    but: Vec<String>,
    #[serde(default)]
    rule: Vec<String>,
}

// ── Compiled language ────────────────────────────────────────────────────────

/// A single Gherkin language with pre-built lookup maps.
#[derive(Debug, Clone)]
pub struct GherkinLanguage {
    pub code: String,
    pub name: String,
    pub native: String,
    /// Maps any step keyword string → StepKeywordType.
    step_map: HashMap<String, StepKeywordType>,
    /// Maps any structural keyword (including the trailing colon, e.g. "功能:") → StructuralType.
    structural_map: HashMap<String, StructuralType>,
    /// All step keyword strings in insertion order (given, when, then, and, but).
    step_keywords: Vec<String>,
    /// All structural keyword+":" strings.
    structural_keywords: Vec<String>,
    /// Primary keyword text for each step type (first entry).
    primary_step: [String; 5],
}

impl GherkinLanguage {
    fn from_raw(code: &str, raw: RawLanguage) -> Self {
        let mut step_map: HashMap<String, StepKeywordType> = HashMap::new();
        let mut step_keywords: Vec<String> = Vec::new();

        fn push_keywords(
            map: &mut HashMap<String, StepKeywordType>,
            list: &mut Vec<String>,
            kw: &[String],
            ty: StepKeywordType,
        ) {
            for s in kw {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    map.entry(trimmed.to_string()).or_insert(ty);
                    if !list.contains(&trimmed.to_string()) {
                        list.push(trimmed.to_string());
                    }
                }
            }
        }

        push_keywords(
            &mut step_map,
            &mut step_keywords,
            &raw.given,
            StepKeywordType::Given,
        );
        push_keywords(
            &mut step_map,
            &mut step_keywords,
            &raw.when,
            StepKeywordType::When,
        );
        push_keywords(
            &mut step_map,
            &mut step_keywords,
            &raw.then,
            StepKeywordType::Then,
        );
        push_keywords(
            &mut step_map,
            &mut step_keywords,
            &raw.and,
            StepKeywordType::And,
        );
        push_keywords(
            &mut step_map,
            &mut step_keywords,
            &raw.but,
            StepKeywordType::But,
        );

        let primary_step = [
            raw.given
                .first()
                .map(|s| s.trim().to_string())
                .unwrap_or_default(),
            raw.when
                .first()
                .map(|s| s.trim().to_string())
                .unwrap_or_default(),
            raw.then
                .first()
                .map(|s| s.trim().to_string())
                .unwrap_or_default(),
            raw.and
                .first()
                .map(|s| s.trim().to_string())
                .unwrap_or_default(),
            raw.but
                .first()
                .map(|s| s.trim().to_string())
                .unwrap_or_default(),
        ];

        let mut structural_map: HashMap<String, StructuralType> = HashMap::new();
        let mut structural_keywords: Vec<String> = Vec::new();

        fn push_structural(
            map: &mut HashMap<String, StructuralType>,
            list: &mut Vec<String>,
            kw: &[String],
            ty: StructuralType,
        ) {
            for s in kw {
                let with_colon = format!("{}:", s.trim());
                map.insert(with_colon.clone(), ty);
                if !list.contains(&with_colon) {
                    list.push(with_colon);
                }
            }
        }

        push_structural(
            &mut structural_map,
            &mut structural_keywords,
            &raw.feature,
            StructuralType::Feature,
        );
        push_structural(
            &mut structural_map,
            &mut structural_keywords,
            &raw.background,
            StructuralType::Background,
        );
        push_structural(
            &mut structural_map,
            &mut structural_keywords,
            &raw.scenario,
            StructuralType::Scenario,
        );
        push_structural(
            &mut structural_map,
            &mut structural_keywords,
            &raw.scenario_outline,
            StructuralType::ScenarioOutline,
        );
        push_structural(
            &mut structural_map,
            &mut structural_keywords,
            &raw.examples,
            StructuralType::Examples,
        );
        push_structural(
            &mut structural_map,
            &mut structural_keywords,
            &raw.rule,
            StructuralType::Rule,
        );

        Self {
            code: code.to_string(),
            name: raw.name,
            native: raw.native,
            step_map,
            structural_map,
            step_keywords,
            structural_keywords,
            primary_step,
        }
    }

    // ── Public API ─────────────────────────────────────────────────────────

    /// Look up a step keyword string → its normalised type.
    pub fn classify_step(&self, word: &str) -> Option<StepKeywordType> {
        self.step_map.get(word).copied()
    }

    /// Check if `trimmed` starts with any step keyword; returns the keyword text
    /// and its type.  The returned keyword text is the exact prefix matched
    /// (preserving the trailing space in the JSON if any).
    pub fn match_step_prefix<'a>(&self, trimmed: &'a str) -> Option<(&'a str, StepKeywordType)> {
        for kw in &self.step_keywords {
            if let Some(rest) = trimmed.strip_prefix(kw.as_str()) {
                if rest.is_empty() || rest.starts_with(' ') {
                    // re-match with the *input* slice to get the exact span
                    let end = kw.len();
                    return Some((&trimmed[..end], self.step_map[kw]));
                }
            }
        }
        None
    }

    /// Iterate all step keyword strings (useful for picker UI).
    pub fn all_step_keywords(&self) -> &[String] {
        &self.step_keywords
    }

    /// Primary display text for a step keyword type.
    pub fn primary_text(&self, ty: StepKeywordType) -> &str {
        let idx = ty as usize;
        if idx < 5 { &self.primary_step[idx] } else { "" }
    }

    /// Check if `trimmed` starts with a structural keyword + ":".
    pub fn match_structural_prefix<'a>(
        &self,
        trimmed: &'a str,
    ) -> Option<(&'a str, StructuralType)> {
        for kw in &self.structural_keywords {
            if let Some(rest) = trimmed.strip_prefix(kw.as_str()) {
                if rest.is_empty() || rest.starts_with(' ') {
                    let end = kw.len();
                    return Some((&trimmed[..end], self.structural_map[kw]));
                }
            }
        }
        None
    }

    /// All structural keyword + ":" strings.
    pub fn all_structural_keywords(&self) -> &[String] {
        &self.structural_keywords
    }

    /// Returns the primary structural keyword + ":" for a type.
    pub fn primary_structural(&self, ty: StructuralType) -> &str {
        // find the first matching entry in structural_map
        self.structural_keywords
            .iter()
            .find(|k| self.structural_map.get(k.as_str()) == Some(&ty))
            .map(|s| s.as_str())
            .unwrap_or("")
    }

    /// Check whether `trimmed` is a step line (starts with any step keyword).
    pub fn is_step_line(&self, trimmed: &str) -> bool {
        self.match_step_prefix(trimmed).is_some()
    }

    /// Check whether `trimmed` is a structural header (starts with structural keyword+":").
    pub fn is_structural(&self, trimmed: &str) -> bool {
        self.match_structural_prefix(trimmed).is_some()
    }
}

// ── Language registry ────────────────────────────────────────────────────────

/// Holds all loaded Gherkin languages.
#[derive(Debug, Clone)]
pub struct GherkinLanguages {
    by_code: HashMap<String, GherkinLanguage>,
}

impl GherkinLanguages {
    /// Build from the embedded JSON data.
    pub fn from_embedded() -> Self {
        let json_str = include_str!("../data/gherkin-languages.json");
        let raw: HashMap<String, RawLanguage> = serde_json::from_str(json_str).unwrap_or_default();

        let mut by_code: HashMap<String, GherkinLanguage> = HashMap::new();
        for (code, raw_lang) in raw {
            let lang = GherkinLanguage::from_raw(&code, raw_lang);
            by_code.insert(code, lang);
        }

        // Ensure English always exists
        if !by_code.contains_key("en") {
            let eng = GherkinLanguage::from_raw(
                "en",
                RawLanguage {
                    name: "English".into(),
                    native: "English".into(),
                    feature: vec!["Feature".into()],
                    background: vec!["Background".into()],
                    scenario: vec!["Scenario".into()],
                    scenario_outline: vec!["Scenario Outline".into()],
                    examples: vec!["Examples".into()],
                    given: vec!["Given".into()],
                    when: vec!["When".into()],
                    then: vec!["Then".into()],
                    and: vec!["And".into()],
                    but: vec!["But".into()],
                    rule: vec!["Rule".into()],
                },
            );
            by_code.insert("en".into(), eng);
        }

        Self { by_code }
    }

    /// Global singleton — initialised once from embedded JSON.
    pub fn global() -> &'static Self {
        static LANG: OnceLock<GherkinLanguages> = OnceLock::new();
        LANG.get_or_init(Self::from_embedded)
    }

    /// Look up a language by code (e.g. "zh-CN", "en"). Falls back to English.
    pub fn get(&self, code: &str) -> &GherkinLanguage {
        self.by_code.get(code).unwrap_or_else(|| {
            self.by_code
                .get("en")
                .expect("English language must always be present")
        })
    }

    /// Detect the language code from feature file content (first line `# language: xx`).
    /// Returns "en" if no directive is found.
    pub fn detect_from_content(content: &str) -> &str {
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("# language:") {
                let code = rest.trim();
                if !code.is_empty() {
                    return Box::leak(code.to_string().into_boxed_str());
                }
            }
            if trimmed.starts_with('#') {
                continue;
            }
            break; // only check before the first non-comment line
        }
        "en"
    }
}
