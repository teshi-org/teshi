//! Markdown image references in requirement documents.
//!
//! Only local files are shown; remote and inline `data:` images are left as
//! source text, since the editor never fetches anything.

use std::path::{Path, PathBuf};

/// One `![alt](target)` reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    /// Alternative text, possibly empty.
    pub alt: String,
    /// Link target without angle brackets or title.
    pub target: String,
}

/// Local image references on one line of Markdown, in order.
pub fn image_refs(line: &str) -> Vec<ImageRef> {
    let mut refs = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find("![") {
        let after_open = &rest[open + 2..];
        let Some(close) = after_open.find("](") else {
            break;
        };
        let alt = &after_open[..close];
        let body = &after_open[close + 2..];
        let Some(end) = body.find(')') else {
            break;
        };
        rest = &body[end + 1..];
        // A `]` inside the alt text means the `![` belonged to other text.
        if alt.contains(']') {
            continue;
        }
        let target = link_target(&body[..end]);
        if !target.is_empty() && !is_remote(target) {
            refs.push(ImageRef {
                alt: alt.to_string(),
                target: target.to_string(),
            });
        }
    }
    refs
}

/// The destination of a link body: `<path with spaces>` or `path "title"`.
fn link_target(body: &str) -> &str {
    let body = body.trim();
    if let Some(quoted) = body.strip_prefix('<') {
        return quoted.split('>').next().unwrap_or_default();
    }
    body.split_whitespace().next().unwrap_or_default()
}

fn is_remote(target: &str) -> bool {
    target.contains("://") || target.starts_with("data:")
}

/// Resolves a reference to an existing file.
///
/// Relative targets are tried against the document's folder first, so a
/// shared requirement store renders the same in every project, and then
/// against the project root.
pub fn resolve(target: &str, document_dir: Option<&Path>, project_root: &Path) -> Option<PathBuf> {
    let path = Path::new(target);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }
    document_dir
        .into_iter()
        .chain(std::iter::once(project_root))
        .map(|base| base.join(path))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_refs_table() {
        let cases: &[(&str, &[(&str, &str)])] = &[
            ("![shot](img/a.png)", &[("shot", "img/a.png")]),
            (
                "x ![](a.png) and ![b](<my b.png> \"title\")",
                &[("", "a.png"), ("b", "my b.png")],
            ),
            ("![remote](https://x.test/a.png)", &[]),
            ("![inline](data:image/png;base64,AA)", &[]),
            ("[link](a.png) ![unterminated](a.png", &[]),
            ("![a](p.png \"t\")", &[("a", "p.png")]),
        ];
        for (line, expected) in cases {
            let refs = image_refs(line);
            let found: Vec<(&str, &str)> = refs
                .iter()
                .map(|r| (r.alt.as_str(), r.target.as_str()))
                .collect();
            assert_eq!(&found, expected, "refs in {line:?}");
        }
    }

    #[test]
    fn test_resolve_prefers_the_document_folder() {
        let store = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        std::fs::write(store.path().join("a.png"), b"x").unwrap();
        std::fs::write(project.path().join("a.png"), b"x").unwrap();
        std::fs::write(project.path().join("b.png"), b"x").unwrap();
        assert_eq!(
            resolve("a.png", Some(store.path()), project.path()),
            Some(store.path().join("a.png"))
        );
        assert_eq!(
            resolve("b.png", Some(store.path()), project.path()),
            Some(project.path().join("b.png"))
        );
        assert_eq!(resolve("c.png", Some(store.path()), project.path()), None);
    }
}
