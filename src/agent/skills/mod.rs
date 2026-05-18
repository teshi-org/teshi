//! Skill/Template system for generation guidance.
//!
//! Skills are loaded from `.tskill` files which contain YAML frontmatter
//! (name, description, keywords) and a markdown body. The registry indexes
//! them by name and provides keyword-based matching against user requests.

mod loader;

use std::collections::HashMap;
use std::path::Path;

pub use loader::parse_tskill_file;

#[derive(Debug, Clone)]
pub struct SkillDefinition {
    pub name: String,
    pub description: String,
    pub keywords: Vec<String>,
    pub content: String,
    #[expect(dead_code)]
    pub path: Option<std::path::PathBuf>,
}

#[derive(Debug)]
pub struct SkillRegistry {
    skills: Vec<SkillDefinition>,
    name_index: HashMap<String, usize>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self {
            skills: Vec::new(),
            name_index: HashMap::new(),
        }
    }

    pub fn register(&mut self, skill: SkillDefinition) {
        let idx = self.skills.len();
        self.name_index.insert(skill.name.clone(), idx);
        self.skills.push(skill);
    }

    pub fn load_from_dir(path: &Path) -> Self {
        let mut registry = Self::new();
        let entries = match std::fs::read_dir(path) {
            Ok(entries) => entries,
            Err(_) => return registry,
        };
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if entry_path.extension().is_some_and(|ext| ext == "tskill")
                && let Some(skill) = parse_tskill_file(&entry_path)
            {
                registry.register(skill);
            }
        }
        registry
    }

    pub fn get(&self, name: &str) -> Option<&SkillDefinition> {
        self.name_index.get(name).map(|&idx| &self.skills[idx])
    }

    /// Match skills by keyword intersection (case-insensitive) and name contains.
    /// Returns up to 5 matches, ranked by number of matched keywords.
    pub fn match_skills(&self, text: &str) -> Vec<&SkillDefinition> {
        let lower = text.to_lowercase();
        // Only use meaningful words (length >= 3) to avoid trivial substring matches
        let text_words: Vec<&str> = lower.split_whitespace().filter(|w| w.len() >= 3).collect();

        let mut scored: Vec<(usize, &SkillDefinition)> = self
            .skills
            .iter()
            .filter_map(|skill| {
                let mut score = 0usize;

                // Name contains match
                if skill.name.to_lowercase().contains(&lower) {
                    score += 10;
                }

                // Keyword intersection
                for kw in &skill.keywords {
                    let kw_lower = kw.to_lowercase();
                    if lower.contains(&kw_lower) {
                        score += 5;
                    } else if text_words
                        .iter()
                        .any(|w| kw_lower.contains(w) || w.contains(&kw_lower))
                    {
                        score += 3;
                    }
                }

                if score > 0 {
                    Some((score, skill))
                } else {
                    None
                }
            })
            .collect();

        // Sort by score descending
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored.truncate(5);
        scored.into_iter().map(|(_, skill)| skill).collect()
    }

    /// Return a formatted catalog string listing all available templates.
    pub fn catalog(&self) -> String {
        if self.skills.is_empty() {
            return String::new();
        }
        let mut out = String::from("Available templates:\n");
        for skill in &self.skills {
            out.push_str(&format!("  - {}: {}\n", skill.name, skill.description));
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_empty_by_default() {
        let reg = SkillRegistry::new();
        assert!(reg.is_empty());
        assert!(reg.catalog().is_empty());
    }

    #[test]
    fn registry_register_and_get() {
        let mut reg = SkillRegistry::new();
        let skill = SkillDefinition {
            name: "test".into(),
            description: "a test skill".into(),
            keywords: vec![],
            content: "# Test\ncontent".into(),
            path: None,
        };
        reg.register(skill);
        assert!(!reg.is_empty());
        assert!(reg.get("test").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn registry_match_skills_by_keyword() {
        let mut reg = SkillRegistry::new();
        reg.register(SkillDefinition {
            name: "auth".into(),
            description: "auth template".into(),
            keywords: vec!["login".into(), "password".into(), "auth".into()],
            content: "".into(),
            path: None,
        });
        reg.register(SkillDefinition {
            name: "crud".into(),
            description: "crud template".into(),
            keywords: vec!["create".into(), "read".into(), "delete".into()],
            content: "".into(),
            path: None,
        });

        let matched = reg.match_skills("I need a login feature");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].name, "auth");
    }

    #[test]
    fn registry_catalog_format() {
        let mut reg = SkillRegistry::new();
        reg.register(SkillDefinition {
            name: "s1".into(),
            description: "desc1".into(),
            keywords: vec![],
            content: "".into(),
            path: None,
        });
        let cat = reg.catalog();
        assert!(cat.contains("s1"));
        assert!(cat.contains("desc1"));
    }
}
