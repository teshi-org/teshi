//! Parser for `.tskill` files with YAML frontmatter + markdown body.
//!
//! Format:
//! ```text
//! ---
//! name: auth-flow
//! description: Login/registration templates
//! keywords: [auth, login, password]
//! ---
//! ... markdown body ...
//! ```

use std::fs;
use std::path::Path;

use super::SkillDefinition;

/// Parse a .tskill file with YAML frontmatter.
/// Returns `None` if the file cannot be read or parsed.
pub fn parse_tskill_file(path: &Path) -> Option<SkillDefinition> {
    let content = fs::read_to_string(path).ok()?;
    let content = content.trim();

    // Split on --- delimiter
    if !content.starts_with("---") {
        return None;
    }
    let rest = content.strip_prefix("---")?.trim_start();
    let (frontmatter, body) = rest.split_once("\n---")?;
    let body = body.trim().to_string();

    // Parse frontmatter lines
    let mut name = String::new();
    let mut description = String::new();
    let mut keywords = Vec::new();

    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("name:") {
            name = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("description:") {
            description = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("keywords:") {
            // Parse simple array: [item1, item2, ...]
            let arr_str = val.trim();
            if arr_str.starts_with('[') && arr_str.ends_with(']') {
                let inner = &arr_str[1..arr_str.len() - 1];
                keywords = inner
                    .split(',')
                    .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        }
    }

    if name.is_empty() || description.is_empty() {
        return None;
    }

    Some(SkillDefinition {
        name,
        description,
        keywords,
        content: body,
        path: Some(path.to_path_buf()),
    })
}
