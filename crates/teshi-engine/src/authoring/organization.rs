//! Project-scoped requirement views. Collections reference shared documents without owning them.
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs, path::Path};
use teshi_core::authoring::{RequirementDocumentMeta, RequirementProjectStatus};

/// A composable, project-local query evaluated against current metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RequirementCondition {
    /// Every child must match.
    All(Vec<Self>),
    /// At least one child must match.
    Any(Vec<Self>),
    /// Negates a condition.
    Not(Box<Self>),
    /// Matches explicit delivery state in the active project.
    Status(RequirementProjectStatus),
    /// Matches a scheduling priority such as P1.
    Priority(String),
    /// Matches an iteration exactly.
    Iteration(String),
    /// Matches title text, ignoring case.
    TitleContains(String),
    /// Matches direct membership of a manual folder.
    Folder(String),
}

/// A manual folder or a saved query, identified independently of its name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequirementCollection {
    /// Stable project-local identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Optional manual parent folder.
    #[serde(default)]
    pub parent: Option<String>,
    /// Manual references; empty for saved queries.
    #[serde(default)]
    pub documents: BTreeSet<String>,
    /// A condition makes this a dynamic collection rather than a manual folder.
    #[serde(default)]
    pub condition: Option<RequirementCondition>,
}

/// Organization for one project and one shared requirement store.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequirementOrganization {
    /// Manual folders and dynamic collections.
    #[serde(default)]
    pub collections: Vec<RequirementCollection>,
}

impl RequirementOrganization {
    /// Resolves a manual folder path such as `AI/ACP` to its stable ID.
    pub fn folder_id(&self, path: &str) -> Option<String> {
        let mut parent = None;
        for name in path.split('/') {
            let folder = self
                .collections
                .iter()
                .find(|c| c.parent == parent && c.name == name && c.condition.is_none())?;
            parent = Some(folder.id.clone());
        }
        parent
    }

    /// Creates missing manual ancestors and returns the leaf ID.
    /// # Errors
    /// Rejects empty path components.
    pub fn ensure_folder(&mut self, path: &str) -> Result<String> {
        self.create_folder(None, path)
    }

    /// Creates missing manual folders for `path` below `parent` and returns the leaf ID.
    ///
    /// Components are trimmed, so `AI / Chat` and `AI/Chat` resolve to the same folders.
    /// # Errors
    /// Rejects empty components, unknown or query parents, and names taken by a query.
    pub fn create_folder(&mut self, parent: Option<&str>, path: &str) -> Result<String> {
        if let Some(id) = parent.filter(|id| !self.is_folder(id)) {
            bail!("unknown manual folder: {id}");
        }
        let mut parent = parent.map(str::to_string);
        for name in path.split('/').map(str::trim) {
            if name.is_empty() {
                bail!("folder path components must not be empty");
            }
            let sibling = self
                .collections
                .iter()
                .find(|c| c.parent == parent && c.name == name);
            match sibling {
                Some(c) if c.condition.is_none() => parent = Some(c.id.clone()),
                Some(_) => bail!("a saved query is already named {name}"),
                None => {
                    let id = uuid::Uuid::new_v4().to_string();
                    self.collections.push(RequirementCollection {
                        id: id.clone(),
                        name: name.to_string(),
                        parent,
                        documents: BTreeSet::new(),
                        condition: None,
                    });
                    parent = Some(id);
                }
            }
        }
        parent.ok_or_else(|| anyhow::anyhow!("folder path is empty"))
    }

    /// Returns the collection with `id`.
    pub fn collection(&self, id: &str) -> Option<&RequirementCollection> {
        self.collections.iter().find(|c| c.id == id)
    }

    /// Returns `true` when `id` names a manual folder rather than a saved query.
    pub fn is_folder(&self, id: &str) -> bool {
        self.collection(id).is_some_and(|c| c.condition.is_none())
    }

    /// Builds the slash-separated display path of a collection, such as `AI/Chat`.
    ///
    /// Returns `None` for unknown IDs or dangling parents; validated organizations always resolve.
    pub fn folder_path(&self, id: &str) -> Option<String> {
        let mut names = Vec::new();
        let mut current = Some(id);
        while let Some(id) = current {
            // Validation caps nesting at 32; the bound also stops cycles in unvalidated input.
            if names.len() > 32 {
                return None;
            }
            let c = self.collection(id)?;
            names.push(c.name.as_str());
            current = c.parent.as_deref();
        }
        names.reverse();
        Some(names.join("/"))
    }

    /// Lists `(id, path)` for every manual folder, sorted by path.
    pub fn folder_paths(&self) -> Vec<(String, String)> {
        let mut paths: Vec<_> = self
            .collections
            .iter()
            .filter(|c| c.condition.is_none())
            .filter_map(|c| Some((c.id.clone(), self.folder_path(&c.id)?)))
            .collect();
        paths.sort_by(|a, b| a.1.cmp(&b.1));
        paths
    }

    /// Returns the manual folders that directly contain `document`.
    pub fn document_folders(&self, document: &str) -> BTreeSet<String> {
        self.collections
            .iter()
            .filter(|c| c.condition.is_none() && c.documents.contains(document))
            .map(|c| c.id.clone())
            .collect()
    }

    /// Replaces the exact manual folder membership of `document`.
    ///
    /// An empty set returns the document to the implicit Inbox.
    /// # Errors
    /// Rejects IDs that are not manual folders without changing membership.
    pub fn set_document_folders(
        &mut self,
        document: &str,
        folders: &BTreeSet<String>,
    ) -> Result<()> {
        if let Some(id) = folders.iter().find(|id| !self.is_folder(id)) {
            bail!("unknown manual folder: {id}");
        }
        for c in self
            .collections
            .iter_mut()
            .filter(|c| c.condition.is_none())
        {
            if folders.contains(&c.id) {
                c.documents.insert(document.to_string());
            } else {
                c.documents.remove(document);
            }
        }
        Ok(())
    }

    /// Adds documents to one manual folder without removing other memberships.
    /// # Errors
    /// Rejects IDs that are not manual folders.
    pub fn add_documents(&mut self, folder: &str, documents: &[String]) -> Result<()> {
        let c = self
            .collections
            .iter_mut()
            .find(|c| c.id == folder && c.condition.is_none())
            .ok_or_else(|| anyhow::anyhow!("unknown manual folder: {folder}"))?;
        c.documents.extend(documents.iter().cloned());
        Ok(())
    }

    /// Renames a folder or saved query in place; its ID and members are unchanged.
    /// # Errors
    /// Rejects unknown IDs, empty names, `/`, and names already used by a sibling.
    pub fn rename_collection(&mut self, id: &str, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() || name.contains('/') {
            bail!("names must be nonempty and must not contain /");
        }
        let parent = self
            .collection(id)
            .ok_or_else(|| anyhow::anyhow!("unknown collection: {id}"))?
            .parent
            .clone();
        if self
            .collections
            .iter()
            .any(|c| c.id != id && c.parent == parent && c.name == name)
        {
            bail!("{name} already exists here");
        }
        if let Some(c) = self.collections.iter_mut().find(|c| c.id == id) {
            c.name = name.to_string();
        }
        Ok(())
    }

    /// IDs of `id` and every collection nested below it.
    fn subtree(&self, id: &str) -> BTreeSet<String> {
        let mut ids = BTreeSet::from([id.to_string()]);
        // Parents may be listed after children, so repeat until no new descendant appears.
        loop {
            let before = ids.len();
            for c in &self.collections {
                if c.parent.as_ref().is_some_and(|p| ids.contains(p)) {
                    ids.insert(c.id.clone());
                }
            }
            if ids.len() == before {
                return ids;
            }
        }
    }

    /// Counts documents that would return to Inbox if `id` and its subfolders were removed.
    pub fn documents_returning_to_inbox(&self, id: &str) -> usize {
        let removed = self.subtree(id);
        let members: BTreeSet<&String> = self
            .collections
            .iter()
            .filter(|c| removed.contains(&c.id))
            .flat_map(|c| &c.documents)
            .collect();
        members
            .into_iter()
            .filter(|doc| {
                !self.collections.iter().any(|c| {
                    !removed.contains(&c.id) && c.condition.is_none() && c.documents.contains(*doc)
                })
            })
            .count()
    }

    /// Removes a saved query, or a folder together with its subfolders.
    ///
    /// Documents are never deleted; ones without another folder appear in Inbox again.
    /// # Errors
    /// Rejects unknown IDs and folders still referenced by a remaining saved query.
    pub fn remove_collection(&mut self, id: &str) -> Result<()> {
        if self.collection(id).is_none() {
            bail!("unknown collection: {id}");
        }
        let removed = self.subtree(id);
        if let Some(query) = self.collections.iter().find(|c| {
            !removed.contains(&c.id)
                && c.condition
                    .as_ref()
                    .is_some_and(|q| condition_references_any(q, &removed))
        }) {
            bail!("saved query {} still filters by this folder", query.name);
        }
        self.collections.retain(|c| !removed.contains(&c.id));
        Ok(())
    }

    /// Creates a root saved query, or updates the query with `id`, and returns its ID.
    /// # Errors
    /// Rejects unknown IDs, folders, empty names, `/`, and sibling name conflicts.
    pub fn save_query(
        &mut self,
        id: Option<&str>,
        name: &str,
        condition: RequirementCondition,
    ) -> Result<String> {
        if let Some(id) = id {
            if self.collection(id).is_none_or(|c| c.condition.is_none()) {
                bail!("unknown saved query: {id}");
            }
            self.rename_collection(id, name)?;
            if let Some(c) = self.collections.iter_mut().find(|c| c.id == id) {
                c.condition = Some(condition);
            }
            return Ok(id.to_string());
        }
        let name = name.trim();
        if name.is_empty() || name.contains('/') {
            bail!("query names must be nonempty and must not contain /");
        }
        if self
            .collections
            .iter()
            .any(|c| c.parent.is_none() && c.name == name)
        {
            bail!("{name} already exists here");
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.collections.push(RequirementCollection {
            id: id.clone(),
            name: name.to_string(),
            parent: None,
            documents: BTreeSet::new(),
            condition: Some(condition),
        });
        Ok(id)
    }

    /// Rewrites `folder=` values given as paths (such as `AI/Chat`) into stable folder IDs.
    ///
    /// Values that already are folder IDs, or match no folder, are kept for validation to judge.
    pub fn condition_with_folder_ids(&self, q: &RequirementCondition) -> RequirementCondition {
        self.map_folders(q, &|v| {
            if self.is_folder(v) {
                v.to_string()
            } else {
                self.folder_id(v).unwrap_or_else(|| v.to_string())
            }
        })
    }

    /// Rewrites `folder=` IDs into readable paths for editing.
    pub fn condition_with_folder_paths(&self, q: &RequirementCondition) -> RequirementCondition {
        self.map_folders(q, &|v| self.folder_path(v).unwrap_or_else(|| v.to_string()))
    }

    fn map_folders(
        &self,
        q: &RequirementCondition,
        f: &dyn Fn(&str) -> String,
    ) -> RequirementCondition {
        match q {
            RequirementCondition::All(v) => {
                RequirementCondition::All(v.iter().map(|q| self.map_folders(q, f)).collect())
            }
            RequirementCondition::Any(v) => {
                RequirementCondition::Any(v.iter().map(|q| self.map_folders(q, f)).collect())
            }
            RequirementCondition::Not(q) => {
                RequirementCondition::Not(Box::new(self.map_folders(q, f)))
            }
            RequirementCondition::Folder(v) => RequirementCondition::Folder(f(v)),
            other => other.clone(),
        }
    }

    /// Returns the root saved query named `name`.
    pub fn root_query_id(&self, name: &str) -> Option<String> {
        self.collections
            .iter()
            .find(|c| c.parent.is_none() && c.name == name.trim() && c.condition.is_some())
            .map(|c| c.id.clone())
    }
    /// Rejects duplicate IDs, dangling parents, cycles and invalid predicates.
    /// # Errors
    /// Returns an error when the configuration is inconsistent.
    pub fn validate(&self) -> Result<()> {
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        for c in &self.collections {
            if c.id.trim().is_empty() || c.name.trim().is_empty() || !ids.insert(&c.id) {
                bail!("collection IDs and names must be nonempty; IDs must be unique");
            }
            if !names.insert((&c.parent, &c.name)) {
                bail!("collection names must be unique within a parent");
            }
            if c.condition.is_some() && !c.documents.is_empty() {
                bail!("query collections cannot contain manual members");
            }
            let mut seen = BTreeSet::from([c.id.as_str()]);
            let mut parent = c.parent.as_deref();
            while let Some(id) = parent {
                if seen.len() > 32 {
                    bail!("folder nesting exceeds 32");
                }
                if !seen.insert(id) {
                    bail!("collection parent cycle");
                }
                let p = self
                    .collections
                    .iter()
                    .find(|p| p.id == id)
                    .ok_or_else(|| anyhow::anyhow!("unknown parent: {id}"))?;
                if p.condition.is_some() {
                    bail!("query collections cannot be parents");
                }
                parent = p.parent.as_deref();
            }
            if let Some(q) = &c.condition {
                self.validate_condition(q, 0)?;
            }
        }
        Ok(())
    }

    /// Checks a condition against this organization before it is saved.
    /// # Errors
    /// Rejects empty groups, excessive nesting, invalid priorities, and unknown folders.
    pub fn check_condition(&self, q: &RequirementCondition) -> Result<()> {
        self.validate_condition(q, 0)
    }

    fn validate_condition(&self, q: &RequirementCondition, depth: usize) -> Result<()> {
        if depth > 32 {
            bail!("condition nesting exceeds 32");
        }
        match q {
            RequirementCondition::All(v) | RequirementCondition::Any(v) => {
                if v.is_empty() {
                    bail!("condition groups must not be empty");
                }
                for q in v {
                    self.validate_condition(q, depth + 1)?;
                }
            }
            RequirementCondition::Not(q) => self.validate_condition(q, depth + 1)?,
            RequirementCondition::Priority(p)
                if !["P0", "P1", "P2", "P3"].contains(&p.as_str()) =>
            {
                bail!("invalid priority")
            }
            RequirementCondition::Folder(id)
                if !self
                    .collections
                    .iter()
                    .any(|c| &c.id == id && c.condition.is_none()) =>
            {
                bail!("unknown manual folder: {id}")
            }
            _ => {}
        }
        Ok(())
    }

    /// Evaluates a saved query using this project's state only.
    pub fn matches(
        &self,
        q: &RequirementCondition,
        doc: &RequirementDocumentMeta,
        project: &str,
    ) -> bool {
        match q {
            RequirementCondition::All(v) => v.iter().all(|q| self.matches(q, doc, project)),
            RequirementCondition::Any(v) => v.iter().any(|q| self.matches(q, doc, project)),
            RequirementCondition::Not(q) => !self.matches(q, doc, project),
            RequirementCondition::Status(s) => doc.projects.get(project) == Some(s),
            RequirementCondition::Priority(p) => doc.priority.is_some_and(|v| v.to_string() == *p),
            RequirementCondition::Iteration(i) => doc.iteration.as_ref() == Some(i),
            RequirementCondition::TitleContains(s) => {
                doc.title.to_lowercase().contains(&s.to_lowercase())
            }
            RequirementCondition::Folder(id) => self
                .collections
                .iter()
                .any(|c| &c.id == id && c.condition.is_none() && c.documents.contains(&doc.id)),
        }
    }
}

fn condition_references_any(q: &RequirementCondition, folders: &BTreeSet<String>) -> bool {
    match q {
        RequirementCondition::All(v) | RequirementCondition::Any(v) => {
            v.iter().any(|q| condition_references_any(q, folders))
        }
        RequirementCondition::Not(q) => condition_references_any(q, folders),
        RequirementCondition::Folder(id) => folders.contains(id),
        _ => false,
    }
}

/// Renders a condition in the text syntax accepted by [`parse_requirement_condition`].
///
/// Returns `None` when the condition needs grouping the text syntax cannot express,
/// such as an `any` nested inside `all`, or values containing separator characters.
pub fn format_requirement_condition(q: &RequirementCondition) -> Option<String> {
    fn atom(q: &RequirementCondition) -> Option<String> {
        let (field, value) = match q {
            RequirementCondition::Not(inner) => return Some(format!("!{}", atom(inner)?)),
            RequirementCondition::Status(s) => (
                "status",
                match s {
                    RequirementProjectStatus::Planned => "planned",
                    RequirementProjectStatus::Implemented => "implemented",
                    RequirementProjectStatus::Verified => "verified",
                },
            ),
            RequirementCondition::Priority(v) => ("priority", v.as_str()),
            RequirementCondition::Iteration(v) => ("iteration", v.as_str()),
            RequirementCondition::TitleContains(v) => ("title", v.as_str()),
            RequirementCondition::Folder(v) => ("folder", v.as_str()),
            RequirementCondition::All(_) | RequirementCondition::Any(_) => return None,
        };
        if value.contains(['&', '|', '!', '=']) || value.trim() != value {
            return None;
        }
        Some(format!("{field}={value}"))
    }
    fn all(q: &RequirementCondition) -> Option<String> {
        match q {
            RequirementCondition::All(v) => {
                Some(v.iter().map(atom).collect::<Option<Vec<_>>>()?.join(" & "))
            }
            _ => atom(q),
        }
    }
    match q {
        RequirementCondition::Any(v) => {
            Some(v.iter().map(all).collect::<Option<Vec<_>>>()?.join(" | "))
        }
        _ => all(q),
    }
}

/// Parses readable query expressions. `&` binds more tightly than `|`; `!` negates an atom.
/// # Errors
/// Rejects unsupported fields, malformed atoms and invalid values.
pub fn parse_requirement_condition(text: &str) -> Result<RequirementCondition> {
    if text.contains('|') {
        return Ok(RequirementCondition::Any(
            text.split('|')
                .map(parse_requirement_condition)
                .collect::<Result<_>>()?,
        ));
    }
    if text.contains('&') {
        return Ok(RequirementCondition::All(
            text.split('&')
                .map(parse_requirement_condition)
                .collect::<Result<_>>()?,
        ));
    }
    let text = text.trim();
    if let Some(rest) = text.strip_prefix('!') {
        return Ok(RequirementCondition::Not(Box::new(
            parse_requirement_condition(rest)?,
        )));
    }
    let (field, value) = text
        .split_once('=')
        .ok_or_else(|| anyhow::anyhow!("expected field=value"))?;
    let value = value.trim();
    if value.is_empty() {
        bail!("condition value must not be empty");
    }
    Ok(match field.trim() {
        "status" => RequirementCondition::Status(match value {
            "planned" => RequirementProjectStatus::Planned,
            "implemented" => RequirementProjectStatus::Implemented,
            "verified" => RequirementProjectStatus::Verified,
            _ => bail!("unknown delivery status"),
        }),
        "priority" if ["P0", "P1", "P2", "P3"].contains(&value) => {
            RequirementCondition::Priority(value.into())
        }
        "iteration" => RequirementCondition::Iteration(value.into()),
        "title" => RequirementCondition::TitleContains(value.into()),
        "folder" => RequirementCondition::Folder(value.into()),
        _ => bail!("unknown condition field or value"),
    })
}

fn organization_path(root: &Path, project: &str) -> Result<std::path::PathBuf> {
    uuid::Uuid::parse_str(project)?;
    Ok(root.join("views").join(format!("{project}.json")))
}

/// Reads project-local organization without creating files.
/// # Errors
/// Reports malformed JSON and invalid configurations instead of hiding them.
pub fn load_requirement_organization(
    root: &Path,
    project: &str,
) -> Result<RequirementOrganization> {
    let path = organization_path(root, project)?;
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RequirementOrganization::default())
        }
        Err(e) => return Err(e.into()),
    };
    let value: RequirementOrganization = serde_json::from_slice(&bytes)?;
    value.validate()?;
    Ok(value)
}

/// Atomically replaces project organization under the requirement store lock.
/// # Errors
/// Rejects invalid configurations or members not linked to this project.
pub fn save_requirement_organization(
    root: &Path,
    project: &str,
    value: &RequirementOrganization,
) -> Result<()> {
    save_requirement_organization_checked(root, project, value, None)
}

/// Replaces organization only if it still matches the caller's snapshot.
/// # Errors
/// Reports a conflict when another editor changed the configuration.
pub fn save_requirement_organization_checked(
    root: &Path,
    project: &str,
    value: &RequirementOrganization,
    expected: Option<&RequirementOrganization>,
) -> Result<()> {
    value.validate()?;
    let path = organization_path(root, project)?;
    super::store::with_requirement_store_lock(root, || {
        if let Some(expected) = expected {
            if &load_requirement_organization(root, project)? != expected {
                bail!("requirement organization changed; reload before saving");
            }
        }
        let index = super::store::read_requirement_index_unlocked(root)?.unwrap_or_default();
        let previous = load_requirement_organization(root, project)?;
        for c in &value.collections {
            for id in &c.documents {
                if !index
                    .documents
                    .iter()
                    .any(|d| &d.id == id && d.projects.contains_key(project))
                    && !previous
                        .collections
                        .iter()
                        .any(|old| old.id == c.id && old.documents.contains(id))
                {
                    bail!("requirement {id} is not linked to this project");
                }
            }
        }
        fs::create_dir_all(path.parent().expect("organization has parent"))?;
        super::store::replace_markdown(&path, &serde_json::to_vec_pretty(value)?)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const PROJECT: &str = "51940e52-6a0b-4cff-8c9c-8954b5e88957";

    #[test]
    fn folders_queries_and_project_states_are_independent() {
        let mut org = RequirementOrganization::default();
        let folder = org.ensure_folder("AI/ACP").unwrap();
        assert_eq!(org.ensure_folder("AI/ACP").unwrap(), folder);
        assert_eq!(org.collections.len(), 2);
        let mut doc = RequirementDocumentMeta::new(
            "doc-1",
            "one.md",
            "ACP runtime",
            teshi_core::authoring::DocumentRevision::new("r1"),
        );
        doc.projects
            .insert(PROJECT.into(), RequirementProjectStatus::Planned);
        doc.projects
            .insert("another-project".into(), RequirementProjectStatus::Verified);
        org.collections[1].documents.insert(doc.id.clone());
        let rule =
            parse_requirement_condition("status=planned & title=ACP | status=implemented").unwrap();
        assert!(org.matches(&rule, &doc, PROJECT));
        assert!(!org.matches(&rule, &doc, "another-project"));
        assert!(org.matches(&RequirementCondition::Folder(folder), &doc, PROJECT));
        assert!(parse_requirement_condition("status=unknown").is_err());
        assert!(parse_requirement_condition("priority=P9").is_err());
        assert!(parse_requirement_condition("title=").is_err());
        assert!(org.ensure_folder("AI//Chat").is_err());
    }

    #[test]
    fn invalid_hierarchies_and_predicates_are_rejected() {
        let mut org = RequirementOrganization::default();
        org.ensure_folder("AI/ACP").unwrap();
        org.collections[0].parent = Some(org.collections[1].id.clone());
        assert!(org.validate().is_err());
        org.collections[0].parent = None;
        org.collections[0].condition = Some(RequirementCondition::All(vec![]));
        assert!(org.validate().is_err());
        org.collections[0].condition = None;
        org.collections[1].condition = Some(RequirementCondition::Folder("missing".into()));
        assert!(org.validate().is_err());
    }

    #[test]
    fn test_folder_operations_keep_ids_and_report_inbox_returns() {
        let mut org = RequirementOrganization::default();
        let ai = org.ensure_folder("AI").unwrap();
        let chat = org.create_folder(Some(&ai), " Chat ").unwrap();
        let other = org.ensure_folder("Other").unwrap();
        assert_eq!(org.folder_path(&chat).as_deref(), Some("AI/Chat"));
        assert!(org.create_folder(Some("missing"), "X").is_err());

        org.set_document_folders("one", &BTreeSet::from([chat.clone()]))
            .unwrap();
        org.set_document_folders("two", &BTreeSet::from([chat.clone(), other.clone()]))
            .unwrap();
        assert_eq!(org.document_folders("two").len(), 2);
        assert!(org
            .set_document_folders("one", &BTreeSet::from(["missing".to_string()]))
            .is_err());
        assert_eq!(org.document_folders("one"), BTreeSet::from([chat.clone()]));

        org.rename_collection(&chat, "Talk").unwrap();
        assert_eq!(org.folder_path(&chat).as_deref(), Some("AI/Talk"));
        assert!(
            org.rename_collection(&other, "AI").is_err(),
            "sibling names stay unique"
        );
        assert!(org.rename_collection(&other, "a/b").is_err());

        assert_eq!(
            org.documents_returning_to_inbox(&ai),
            1,
            "only the document without another folder returns to Inbox"
        );
        org.save_query(None, "Mine", RequirementCondition::Folder(chat.clone()))
            .unwrap();
        assert!(
            org.remove_collection(&ai).is_err(),
            "queries must not dangle"
        );
        let query = org.root_query_id("Mine").unwrap();
        org.remove_collection(&query).unwrap();
        org.remove_collection(&ai).unwrap();
        assert_eq!(org.folder_paths(), vec![(other, "Other".to_string())]);
        org.validate().unwrap();
    }

    #[test]
    fn test_save_query_creates_updates_and_rejects_conflicts() {
        let mut org = RequirementOrganization::default();
        org.ensure_folder("AI").unwrap();
        let planned = RequirementCondition::Status(RequirementProjectStatus::Planned);
        let id = org.save_query(None, "Pending", planned.clone()).unwrap();
        assert!(org.save_query(None, "Pending", planned.clone()).is_err());
        assert!(org.save_query(None, "AI", planned.clone()).is_err());
        assert_eq!(org.collections.len(), 2, "rejected queries are not added");
        let verified = RequirementCondition::Status(RequirementProjectStatus::Verified);
        assert_eq!(
            org.save_query(Some(&id), "Done", verified.clone()).unwrap(),
            id
        );
        assert_eq!(org.collection(&id).unwrap().condition, Some(verified));
        let folder = org.folder_id("AI").unwrap();
        assert!(org.save_query(Some(&folder), "X", planned).is_err());
        org.validate().unwrap();
    }

    #[test]
    fn test_condition_text_round_trips_with_folder_paths() {
        let mut org = RequirementOrganization::default();
        let chat = org.ensure_folder("AI/Chat").unwrap();
        for text in [
            "status=planned",
            "status=planned & priority=P1 | !title=Draft",
            "folder=AI/Chat & iteration=Sprint 1",
        ] {
            let parsed = org.condition_with_folder_ids(&parse_requirement_condition(text).unwrap());
            org.validate_condition(&parsed, 0).unwrap();
            let shown = org.condition_with_folder_paths(&parsed);
            assert_eq!(format_requirement_condition(&shown).as_deref(), Some(text));
        }
        assert_eq!(
            org.condition_with_folder_ids(&RequirementCondition::Folder("AI/Chat".into())),
            RequirementCondition::Folder(chat)
        );
        let nested = RequirementCondition::All(vec![RequirementCondition::Any(vec![
            RequirementCondition::Iteration("a".into()),
        ])]);
        assert_eq!(format_requirement_condition(&nested), None);
    }

    #[test]
    fn persistence_rejects_stale_writes_and_cross_project_members() {
        let root = tempfile::tempdir().unwrap();
        super::super::store::initialize_requirement_store(root.path()).unwrap();
        let empty = load_requirement_organization(root.path(), PROJECT).unwrap();
        let mut first = empty.clone();
        first.ensure_folder("AI").unwrap();
        save_requirement_organization_checked(root.path(), PROJECT, &first, Some(&empty)).unwrap();
        assert_eq!(
            load_requirement_organization(root.path(), PROJECT).unwrap(),
            first
        );
        assert!(
            save_requirement_organization_checked(root.path(), PROJECT, &empty, Some(&empty))
                .is_err()
        );
        first.collections[0]
            .documents
            .insert("unknown-document".into());
        assert!(save_requirement_organization(root.path(), PROJECT, &first).is_err());
        assert!(load_requirement_organization(root.path(), "../escape").is_err());
        assert!(
            load_requirement_organization(root.path(), "5493e79f-06a8-4381-add3-d3d336ac94db")
                .unwrap()
                .collections
                .is_empty()
        );
    }
}
