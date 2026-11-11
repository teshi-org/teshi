//! Project organization interactions on the Requirements tree.
//!
//! The Space action menu lists only operations valid for the selected row. Folder and
//! query edits use typed forms and reload the organization file before writing, so a
//! stale in-memory snapshot never overwrites edits made by the CLI or another window.

use std::collections::HashMap;

use teshi_core::authoring::{
    RequirementDocumentMeta, RequirementGroupMode, RequirementProjectFilter,
    RequirementProjectStatus, normalize_tag_name,
};
use teshi_engine::organization::{
    RequirementOrganization, format_requirement_condition, load_requirement_organization,
    parse_requirement_condition, save_requirement_organization_checked,
};
use tui_tree_widget::TreeItem;

use super::{
    AuthoringUiState, RequirementsOverlay, TREE_DOC_PREFIX, requirement_document_tree_label,
};

/// Tree node of the implicit Inbox (documents without a manual folder).
pub const TREE_INBOX_NODE: &str = "req-dir:inbox";
/// Tree node grouping every root saved query.
pub const TREE_QUERIES_NODE: &str = "req-dir:queries";
/// Tree node prefix for manual folders and saved queries, followed by the stable ID.
pub const TREE_COLLECTION_PREFIX: &str = "req-dir:collection:";

/// Cursor movement inside a [`LineInput`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorMotion {
    Left,
    Right,
    Home,
    End,
}

/// Single-line text input with a cursor counted in Unicode scalar values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineInput {
    text: String,
    cursor: usize,
}

impl LineInput {
    /// Creates an input holding `text` with the cursor at the end.
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.chars().count();
        Self { text, cursor }
    }

    /// Current text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Text before and after the cursor, used to draw the caret.
    pub fn split_at_cursor(&self) -> (&str, &str) {
        self.text.split_at(self.byte_offset(self.cursor))
    }

    fn byte_offset(&self, chars: usize) -> usize {
        self.text
            .char_indices()
            .nth(chars)
            .map_or(self.text.len(), |(index, _)| index)
    }

    /// Inserts `ch` at the cursor.
    pub fn insert(&mut self, ch: char) {
        let offset = self.byte_offset(self.cursor);
        self.text.insert(offset, ch);
        self.cursor += 1;
    }

    /// Removes the character before the cursor.
    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let offset = self.byte_offset(self.cursor);
            self.text.remove(offset);
        }
    }

    /// Removes the character under the cursor.
    pub fn delete(&mut self) {
        if self.cursor < self.text.chars().count() {
            let offset = self.byte_offset(self.cursor);
            self.text.remove(offset);
        }
    }

    /// Moves the cursor without editing.
    pub fn move_cursor(&mut self, motion: CursorMotion) {
        let len = self.text.chars().count();
        self.cursor = match motion {
            CursorMotion::Left => self.cursor.saturating_sub(1),
            CursorMotion::Right => (self.cursor + 1).min(len),
            CursorMotion::Home => 0,
            CursorMotion::End => len,
        };
    }
}

/// Kind of row selected in the Requirements tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeTarget {
    /// A requirement document, by stable ID.
    Document(String),
    /// Documents of the current project that are in no manual folder.
    Inbox,
    /// A manual folder, by stable collection ID.
    Folder(String),
    /// The group listing saved queries.
    Queries,
    /// A saved query, by stable collection ID.
    Query(String),
    /// A path or iteration group outside the Folders view.
    Group,
}

/// Operation offered by the Space action menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCommand {
    OpenEditor,
    EditFolders,
    EditTags,
    ToggleLink,
    EditIteration,
    EditPriority,
    DeleteDocument,
    NewFolder,
    NewRequirement,
    Rename,
    Remove,
    NewQuery,
    EditQuery,
}

/// One row of the Space action menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuItem {
    /// Operation run when the row is chosen.
    pub command: MenuCommand,
    /// Visible label.
    pub label: &'static str,
    /// Equivalent tree shortcut, shown for learning and accepted inside the menu.
    pub shortcut: Option<char>,
    /// Display text for the shortcut column.
    pub hint: &'static str,
}

const fn item(
    command: MenuCommand,
    label: &'static str,
    shortcut: Option<char>,
    hint: &'static str,
) -> MenuItem {
    MenuItem {
        command,
        label,
        shortcut,
        hint,
    }
}

/// Text form for folder and query edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrganizationForm {
    /// Create a folder (or `a/b` path) below `parent`, or at the root.
    NewFolder {
        parent: Option<String>,
        parent_path: Option<String>,
        name: LineInput,
    },
    /// Rename a folder or saved query; IDs and members are kept.
    Rename {
        collection_id: String,
        query: bool,
        name: LineInput,
    },
    /// Create a saved query, or edit the one with `query_id`.
    Query {
        query_id: Option<String>,
        name: LineInput,
        condition: LineInput,
        /// `true` while the condition field has the cursor.
        condition_focused: bool,
    },
}

impl OrganizationForm {
    fn focused_input(&mut self) -> &mut LineInput {
        match self {
            Self::NewFolder { name, .. } | Self::Rename { name, .. } => name,
            Self::Query {
                name,
                condition,
                condition_focused,
                ..
            } => {
                if *condition_focused {
                    condition
                } else {
                    name
                }
            }
        }
    }
}

const SCOPE_REQUIRED: &str =
    "Folders and saved queries belong to Current project; press o to switch scope";

impl AuthoringUiState {
    fn current_project(&self) -> Result<&str, String> {
        match &self.project_filter {
            RequirementProjectFilter::Project(project) => Ok(project),
            _ => Err(SCOPE_REQUIRED.into()),
        }
    }

    /// Whether the tree shows project folders and saved queries.
    pub fn folders_view_active(&self) -> bool {
        matches!(self.project_filter, RequirementProjectFilter::Project(_))
            && self.group_mode == RequirementGroupMode::Path
    }

    /// Name of the active tree view: Folders, Path, or Iteration.
    pub fn view_label(&self) -> &'static str {
        match (self.group_mode, self.is_project_scope()) {
            (RequirementGroupMode::Iteration, _) => "Iteration",
            (RequirementGroupMode::Path, true) => "Folders",
            (RequirementGroupMode::Path, false) => "Path",
        }
    }

    /// Whether the tree is limited to the current project (the only scope with delivery state).
    pub fn is_project_scope(&self) -> bool {
        matches!(self.project_filter, RequirementProjectFilter::Project(_))
    }

    /// Tree title listing every active view setting in key order: `o`, `g`, `i`, `H`.
    pub fn tree_status_title(&self) -> String {
        let iteration = match &self.iteration_filter {
            teshi_core::authoring::RequirementIterationFilter::All => "All",
            teshi_core::authoring::RequirementIterationFilter::Unassigned => {
                teshi_core::authoring::UNASSIGNED_ITERATION_LABEL
            }
            teshi_core::authoring::RequirementIterationFilter::Named(name) => name.as_str(),
        };
        let mut title = format!(
            "{} · {} · Iteration: {iteration}",
            self.project_scope_label(),
            self.view_label()
        );
        // Delivery state is recorded per project, so the filter only exists in project scope.
        if self.is_project_scope() {
            title.push_str(if self.show_completed {
                " · All states"
            } else {
                " · Pending only"
            });
        }
        title
    }

    /// Toggles visibility of implemented and verified requirements.
    ///
    /// # Errors
    /// Outside Current project scope, where delivery state is undefined.
    pub fn toggle_completed(&mut self) -> Result<bool, String> {
        self.current_project()?;
        self.show_completed = !self.show_completed;
        self.rebuild_tree();
        Ok(self.show_completed)
    }

    /// Classifies the selected tree row.
    pub fn tree_target(&self) -> Option<TreeTarget> {
        let node = self.tree_state.selected().last()?;
        Some(if let Some(id) = node.strip_prefix(TREE_DOC_PREFIX) {
            TreeTarget::Document(id.to_string())
        } else if node == TREE_INBOX_NODE {
            TreeTarget::Inbox
        } else if node == TREE_QUERIES_NODE {
            TreeTarget::Queries
        } else if let Some(id) = node.strip_prefix(TREE_COLLECTION_PREFIX) {
            match self
                .organization
                .as_ref()
                .and_then(|org| org.collection(id))
            {
                Some(c) if c.condition.is_some() => TreeTarget::Query(id.to_string()),
                Some(_) => TreeTarget::Folder(id.to_string()),
                None => TreeTarget::Group,
            }
        } else {
            TreeTarget::Group
        })
    }

    /// Lists the operations valid for `target` in the current scope.
    pub fn menu_items(&self, target: &TreeTarget) -> Vec<MenuItem> {
        use MenuCommand::*;
        let new_requirement = item(NewRequirement, "New requirement", None, "Ctrl+n");
        match target {
            TreeTarget::Document(_) => {
                let mut items = vec![item(OpenEditor, "Open in editor", None, "Enter")];
                if self.folders_view_active() {
                    items.push(item(EditFolders, "Folders…", None, ""));
                }
                items.extend([
                    item(EditTags, "Tags…", None, ""),
                    item(ToggleLink, "Link / unlink project", Some('b'), "b"),
                    item(EditIteration, "Set iteration…", Some('I'), "I"),
                    item(EditPriority, "Set priority…", Some('p'), "p"),
                    item(DeleteDocument, "Delete requirement…", Some('d'), "d"),
                ]);
                items
            }
            TreeTarget::Inbox => vec![item(NewFolder, "New folder…", None, ""), new_requirement],
            TreeTarget::Folder(_) => vec![
                item(NewFolder, "New subfolder…", None, ""),
                item(NewRequirement, "New requirement here", None, "Ctrl+n"),
                item(Rename, "Rename…", None, ""),
                item(Remove, "Delete folder…", None, ""),
            ],
            TreeTarget::Queries => vec![item(NewQuery, "New saved query…", None, "")],
            TreeTarget::Query(_) => vec![
                item(EditQuery, "Edit query…", None, ""),
                item(Rename, "Rename…", None, ""),
                item(Remove, "Delete query…", None, ""),
            ],
            TreeTarget::Group => vec![new_requirement],
        }
    }

    /// Popup title naming the row the menu acts on.
    pub fn menu_title(&self, target: &TreeTarget) -> String {
        let collection = |id: &str| {
            self.organization
                .as_ref()
                .and_then(|org| org.folder_path(id))
                .unwrap_or_default()
        };
        match target {
            TreeTarget::Document(id) => {
                let title = self
                    .artifacts
                    .as_ref()
                    .and_then(|a| a.index.documents.iter().find(|d| &d.id == id))
                    .map_or(id.as_str(), |d| d.title.as_str());
                format!(" Requirement: {title} ")
            }
            TreeTarget::Inbox => " Inbox ".into(),
            TreeTarget::Folder(id) => format!(" Folder: {} ", collection(id)),
            TreeTarget::Queries => " Saved queries ".into(),
            TreeTarget::Query(id) => format!(" Query: {} ", collection(id)),
            TreeTarget::Group => " Requirements ".into(),
        }
    }

    /// Opens the action menu for the selected tree row.
    ///
    /// # Errors
    /// When no tree row is selected.
    pub fn open_action_menu(&mut self) -> Result<(), String> {
        let target = self
            .tree_target()
            .ok_or("Select a row in the requirement tree first")?;
        self.overlay = RequirementsOverlay::ActionMenu {
            target,
            selection: 0,
        };
        Ok(())
    }

    /// Returns the open menu's target and highlighted command.
    pub fn menu_selection(&self) -> Option<(TreeTarget, MenuCommand)> {
        let RequirementsOverlay::ActionMenu { target, selection } = &self.overlay else {
            return None;
        };
        let command = self.menu_items(target).get(*selection)?.command;
        Some((target.clone(), command))
    }

    /// Returns the open menu's command bound to shortcut `ch`.
    pub fn menu_shortcut(&self, ch: char) -> Option<(TreeTarget, MenuCommand)> {
        let RequirementsOverlay::ActionMenu { target, .. } = &self.overlay else {
            return None;
        };
        let command = self
            .menu_items(target)
            .into_iter()
            .find(|item| item.shortcut == Some(ch))?
            .command;
        Some((target.clone(), command))
    }

    /// Number of rows in the open menu or folder picker, for list navigation.
    pub fn organization_list_len(&self) -> Option<usize> {
        match &self.overlay {
            RequirementsOverlay::ActionMenu { target, .. } => Some(self.menu_items(target).len()),
            RequirementsOverlay::FolderPicker { options, .. } => Some(options.len()),
            // Known tags plus the trailing "New tag…" row.
            RequirementsOverlay::TagPicker { options, .. } => Some(options.len() + 1),
            _ => None,
        }
    }

    /// Opens the tag picker for a document row.
    ///
    /// Tags are document-global, so this works in every scope, including
    /// All projects where folders and saved queries are unavailable.
    ///
    /// # Errors
    /// When the row is not a document or the row is no longer in the index.
    pub fn open_tag_picker(&mut self, target: &TreeTarget) -> Result<(), String> {
        let TreeTarget::Document(id) = target else {
            return Err("Select a requirement row first".into());
        };
        let index = &self
            .artifacts
            .as_ref()
            .ok_or("Requirement store is not loaded")?
            .index;
        let document = index
            .documents
            .iter()
            .find(|doc| &doc.id == id)
            .ok_or("This requirement no longer exists")?;
        self.overlay = RequirementsOverlay::TagPicker {
            document_id: id.clone(),
            options: index.discovered_tags(),
            checked: document.tags.clone(),
            selection: 0,
            new_tag: None,
        };
        Ok(())
    }

    /// Checks or unchecks the highlighted tag, ignoring the "New tag…" row.
    pub fn toggle_tag_choice(&mut self) {
        if let RequirementsOverlay::TagPicker {
            options,
            checked,
            selection,
            new_tag,
            ..
        } = &mut self.overlay
            && new_tag.is_none()
            && let Some(tag) = options.get(*selection)
        {
            let tag = tag.clone();
            if !checked.remove(&tag) {
                checked.insert(tag);
            }
        }
    }

    /// Applies the highlighted tag-picker row.
    ///
    /// The "New tag…" row opens the inline field; Enter in that field commits the
    /// typed name and keeps the picker open; Enter anywhere else saves the set.
    ///
    /// # Errors
    /// When the tag name is empty, contains control characters, or the store
    /// write fails.
    pub(crate) fn confirm_tag_picker(&mut self) -> Result<String, String> {
        let RequirementsOverlay::TagPicker {
            document_id,
            options,
            checked,
            selection,
            new_tag,
        } = self.overlay.clone()
        else {
            return Err("No tag picker is open".into());
        };
        if let Some(raw) = new_tag {
            let tag = normalize_tag_name(&raw).map_err(|err| err.to_string())?;
            if let RequirementsOverlay::TagPicker {
                options,
                checked,
                selection,
                new_tag,
                ..
            } = &mut self.overlay
            {
                if !options.contains(&tag) {
                    options.push(tag.clone());
                    options.sort();
                }
                checked.insert(tag.clone());
                *new_tag = None;
                *selection = options.iter().position(|item| item == &tag).unwrap_or(0);
            }
            return Ok(format!("Added tag {tag}"));
        }
        if selection == options.len() {
            if let RequirementsOverlay::TagPicker { new_tag, .. } = &mut self.overlay {
                *new_tag = Some(String::new());
            }
            return Ok(String::new());
        }
        let tags: Vec<String> = checked.into_iter().collect();
        let index = teshi_engine::set_requirement_documents_tags(
            &self.requirements_root,
            &[&document_id],
            &tags,
        )
        .map_err(|err| err.to_string())?;
        let saved = index
            .documents
            .iter()
            .find(|doc| doc.id == document_id)
            .map(|doc| doc.tags.clone())
            .unwrap_or_default();
        if let Some(artifacts) = self.artifacts.as_mut() {
            if let Some(meta) = artifacts
                .index
                .documents
                .iter_mut()
                .find(|d| d.id == document_id)
            {
                meta.tags = saved.clone();
            }
            if let Some(doc) = artifacts
                .documents
                .iter_mut()
                .find(|d| d.meta.id == document_id)
            {
                doc.meta.tags = saved;
            }
        }
        self.overlay = RequirementsOverlay::None;
        self.rebuild_tree();
        self.note_store_write();
        Ok(format!("Updated tags of {document_id}"))
    }

    /// Opens the form, picker, or confirmation behind an organization menu command.
    ///
    /// Commands handled by existing tree actions (link, iteration, ...) are ignored here.
    /// # Errors
    /// Outside Current project scope, or when the organization file cannot be read.
    pub fn open_organization_command(
        &mut self,
        command: MenuCommand,
        target: &TreeTarget,
    ) -> Result<(), String> {
        let project = self.current_project()?.to_string();
        let org = load_requirement_organization(&self.requirements_root, &project)
            .map_err(|e| format!("Requirement organization: {e}"))?;
        let collection_id = match target {
            TreeTarget::Folder(id) | TreeTarget::Query(id) => Some(id.clone()),
            _ => None,
        };
        self.overlay = match (command, target) {
            (MenuCommand::EditFolders, TreeTarget::Document(id)) => {
                RequirementsOverlay::FolderPicker {
                    document_id: id.clone(),
                    checked: org.document_folders(id),
                    options: org.folder_paths(),
                    selection: 0,
                }
            }
            (MenuCommand::NewFolder, _) => {
                let parent_path = collection_id.as_deref().and_then(|id| org.folder_path(id));
                RequirementsOverlay::OrganizationForm(OrganizationForm::NewFolder {
                    parent: collection_id,
                    parent_path,
                    name: LineInput::default(),
                })
            }
            (MenuCommand::Rename, TreeTarget::Folder(id) | TreeTarget::Query(id)) => {
                let c = org.collection(id).ok_or("This item no longer exists")?;
                RequirementsOverlay::OrganizationForm(OrganizationForm::Rename {
                    collection_id: id.clone(),
                    query: c.condition.is_some(),
                    name: LineInput::new(c.name.clone()),
                })
            }
            (MenuCommand::Remove, TreeTarget::Folder(id) | TreeTarget::Query(id)) => {
                let c = org.collection(id).ok_or("This item no longer exists")?;
                RequirementsOverlay::RemoveCollection {
                    collection_id: id.clone(),
                    name: org.folder_path(id).unwrap_or_else(|| c.name.clone()),
                    query: c.condition.is_some(),
                    inbox_returns: org.documents_returning_to_inbox(id),
                }
            }
            (MenuCommand::NewQuery, _) => {
                RequirementsOverlay::OrganizationForm(OrganizationForm::Query {
                    query_id: None,
                    name: LineInput::default(),
                    condition: LineInput::new("status=planned"),
                    condition_focused: false,
                })
            }
            (MenuCommand::EditQuery, TreeTarget::Query(id)) => {
                let c = org.collection(id).ok_or("This item no longer exists")?;
                let text = c
                    .condition
                    .as_ref()
                    .and_then(|q| format_requirement_condition(&org.condition_with_folder_paths(q)))
                    .ok_or(
                        "This query uses nesting the editor cannot show; \
                         edit it with `teshi requirements organize --file`",
                    )?;
                RequirementsOverlay::OrganizationForm(OrganizationForm::Query {
                    query_id: Some(id.clone()),
                    name: LineInput::new(c.name.clone()),
                    condition: LineInput::new(text),
                    condition_focused: true,
                })
            }
            _ => return Ok(()),
        };
        Ok(())
    }

    /// Edits the focused text field of an organization form.
    pub(super) fn organization_form_input(&mut self) -> Option<&mut LineInput> {
        match &mut self.overlay {
            RequirementsOverlay::OrganizationForm(form) => Some(form.focused_input()),
            _ => None,
        }
    }

    /// Moves the cursor of the focused text field in a Requirements text overlay.
    pub fn overlay_move_cursor(&mut self, motion: CursorMotion) {
        if let Some(input) = self.organization_form_input() {
            input.move_cursor(motion);
        }
    }

    /// Deletes the character under the cursor in a Requirements text overlay.
    pub fn overlay_delete(&mut self) {
        if let Some(input) = self.organization_form_input() {
            input.delete();
        }
    }

    /// Switches between the name and condition fields of the query form.
    pub(super) fn organization_form_switch_field(&mut self) -> bool {
        if let RequirementsOverlay::OrganizationForm(OrganizationForm::Query {
            condition_focused,
            ..
        }) = &mut self.overlay
        {
            *condition_focused = !*condition_focused;
            return true;
        }
        false
    }

    /// Checks or unchecks the highlighted folder in the folder picker.
    pub fn toggle_folder_choice(&mut self) {
        if let RequirementsOverlay::FolderPicker {
            options,
            checked,
            selection,
            ..
        } = &mut self.overlay
            && let Some((id, _)) = options.get(*selection)
            && !checked.remove(id)
        {
            checked.insert(id.clone());
        }
    }

    /// Live result of the query form: matching document count or the parse error.
    ///
    /// Counts the same documents the tree shows, so the number matches what the
    /// saved query will list under the current filters.
    pub fn query_form_preview(&self) -> Option<Result<usize, String>> {
        let RequirementsOverlay::OrganizationForm(OrganizationForm::Query { condition, .. }) =
            &self.overlay
        else {
            return None;
        };
        let project = self.current_project().ok()?;
        let org = self.organization.clone().unwrap_or_default();
        let result = parse_requirement_condition(condition.text())
            .map(|q| org.condition_with_folder_ids(&q))
            .and_then(|q| org.check_condition(&q).map(|()| q))
            .map(|q| {
                self.project_view_documents()
                    .iter()
                    .filter(|d| org.matches(&q, d, project))
                    .count()
            });
        Some(result.map_err(|e| e.to_string()))
    }

    /// Applies an organization form, picker, or removal confirmation.
    ///
    /// Returns `None` for overlays owned by other features. On failure the overlay stays
    /// open so the input can be corrected.
    pub(crate) fn confirm_organization_overlay(&mut self) -> Option<Result<String, String>> {
        let overlay = self.overlay.clone();
        let result = match overlay {
            RequirementsOverlay::FolderPicker {
                document_id,
                checked,
                ..
            } => self
                .update_organization(|org| org.set_document_folders(&document_id, &checked))
                .map(|()| {
                    if checked.is_empty() {
                        format!("Moved {document_id} to Inbox")
                    } else {
                        format!("Updated folders of {document_id}")
                    }
                }),
            RequirementsOverlay::OrganizationForm(OrganizationForm::NewFolder {
                parent,
                name,
                ..
            }) => self
                .update_organization(|org| {
                    org.create_folder(parent.as_deref(), name.text()).map(drop)
                })
                .map(|()| format!("Created folder {}", name.text().trim())),
            RequirementsOverlay::OrganizationForm(OrganizationForm::Rename {
                collection_id,
                name,
                ..
            }) => self
                .update_organization(|org| org.rename_collection(&collection_id, name.text()))
                .map(|()| format!("Renamed to {}", name.text().trim())),
            RequirementsOverlay::OrganizationForm(OrganizationForm::Query {
                query_id,
                name,
                condition,
                ..
            }) => self
                .update_organization(|org| {
                    let q = org
                        .condition_with_folder_ids(&parse_requirement_condition(condition.text())?);
                    org.save_query(query_id.as_deref(), name.text(), q)
                        .map(drop)
                })
                .map(|()| format!("Saved query {}", name.text().trim())),
            RequirementsOverlay::RemoveCollection {
                collection_id,
                name,
                ..
            } => self
                .update_organization(|org| org.remove_collection(&collection_id))
                .map(|()| format!("Deleted {name}")),
            _ => return None,
        };
        if result.is_ok() {
            self.overlay = RequirementsOverlay::None;
        }
        Some(result)
    }

    /// Reloads, edits, and conditionally saves the organization, then rebuilds the tree.
    fn update_organization(
        &mut self,
        edit: impl FnOnce(&mut RequirementOrganization) -> anyhow::Result<()>,
    ) -> Result<(), String> {
        let project = self.current_project()?.to_string();
        let root = self.requirements_root.clone();
        let result = (|| {
            let mut org = load_requirement_organization(&root, &project)?;
            let expected = org.clone();
            edit(&mut org)?;
            save_requirement_organization_checked(&root, &project, &org, Some(&expected))
        })();
        result.map_err(|e| e.to_string())?;
        self.rebuild_tree();
        Ok(())
    }

    /// Folder that receives a new requirement created while it is selected.
    pub(super) fn selected_folder(&self) -> Option<String> {
        match self.tree_target()? {
            TreeTarget::Folder(id) if self.folders_view_active() => Some(id),
            _ => None,
        }
    }

    /// Files a just-saved draft into the folder it was created in.
    ///
    /// # Errors
    /// When the organization cannot be updated; the Markdown is already saved by then.
    pub(super) fn file_saved_draft(&mut self, document_id: &str) -> anyhow::Result<()> {
        let Some((draft, folder)) = self.draft_folder.take() else {
            return Ok(());
        };
        if draft != document_id {
            return Ok(());
        }
        self.update_organization(|org| org.add_documents(&folder, &[draft]))
            .map_err(|e| anyhow::anyhow!("saved, but could not add it to the folder: {e}"))
    }

    /// Project documents visible under the iteration and completed filters.
    pub(super) fn project_view_documents(&self) -> Vec<RequirementDocumentMeta> {
        let RequirementProjectFilter::Project(project) = &self.project_filter else {
            return Vec::new();
        };
        self.project_filtered_index()
            .documents
            .into_iter()
            .filter(|d| d.matches_iteration_filter(&self.iteration_filter))
            .filter(|d| {
                self.show_completed
                    || d.projects.get(project) == Some(&RequirementProjectStatus::Planned)
            })
            .collect()
    }

    /// Loads the organization and builds the Folders view.
    ///
    /// A project that has no identity yet has no folders either. Keep the normal
    /// empty state instead of surfacing a store-level ID validation error, which
    /// would hide the artifact list and the create hint.
    pub(super) fn rebuild_folders_tree(&mut self, project: &str) {
        if project.is_empty() {
            self.tree_items = Vec::new();
            self.tree_labels.clear();
            self.organization = None;
            self.project_identity_error = None;
            return;
        }
        match load_requirement_organization(&self.requirements_root, project) {
            Ok(org) => {
                if self
                    .project_identity_error
                    .as_deref()
                    .is_some_and(|e| e.starts_with("Requirement organization:"))
                {
                    self.project_identity_error = None;
                }
                let mut shown = org.clone();
                // An unsaved draft cannot be stored yet, but it should appear where it was created.
                if let (Some(draft), Some((id, folder))) =
                    (&self.draft_document_id, &self.draft_folder)
                    && draft == id
                {
                    let _ = shown.add_documents(folder, std::slice::from_ref(id));
                }
                let docs = self.project_view_documents();
                let (items, labels) = build_organization_tree(&docs, &shown, project);
                self.tree_items = items;
                self.tree_labels = labels;
                self.organization = Some(org);
            }
            Err(error) => {
                self.tree_items = Vec::new();
                self.tree_labels.clear();
                self.project_identity_error = Some(format!("Requirement organization: {error}"));
            }
        }
    }

    /// Refreshes collection changes made by another CLI or UI without replacing dirty Markdown.
    pub fn poll_organization_changes(&mut self) {
        let RequirementProjectFilter::Project(project) = &self.project_filter else {
            return;
        };
        if project.is_empty() {
            return;
        }
        let org_error = self
            .project_identity_error
            .as_deref()
            .is_some_and(|e| e.starts_with("Requirement organization:"));
        match load_requirement_organization(&self.requirements_root, project) {
            Ok(org) if org_error || self.organization.as_ref() != Some(&org) => self.rebuild_tree(),
            Ok(_) => {}
            Err(error) => {
                self.project_identity_error = Some(format!("Requirement organization: {error}"))
            }
        }
    }
}

/// Builds Inbox, manual folders, and the Saved queries group.
///
/// Folders list only direct members; parents never aggregate descendants. The returned
/// map holds each node's visible label for width measurement.
pub(super) fn build_organization_tree(
    docs: &[RequirementDocumentMeta],
    org: &RequirementOrganization,
    project: &str,
) -> (Vec<TreeItem<'static, String>>, HashMap<String, String>) {
    let mut builder = OrganizationTreeBuilder {
        docs,
        org,
        project,
        labels: HashMap::new(),
    };
    let inbox: Vec<_> = docs
        .iter()
        .filter(|d| {
            !org.collections
                .iter()
                .any(|c| c.condition.is_none() && c.documents.contains(&d.id))
        })
        .collect();
    let inbox_label = format!("Inbox ({})", inbox.len());
    let inbox_items = inbox.into_iter().map(|d| builder.leaf(d)).collect();
    let mut items = vec![builder.node(TREE_INBOX_NODE.into(), inbox_label, inbox_items)];
    items.extend(builder.children(None));
    let queries = org
        .collections
        .iter()
        .filter(|c| c.parent.is_none() && c.condition.is_some())
        .map(|c| builder.collection(c))
        .collect();
    items.push(builder.node(TREE_QUERIES_NODE.into(), "Saved queries".into(), queries));
    (items, builder.labels)
}

struct OrganizationTreeBuilder<'a> {
    docs: &'a [RequirementDocumentMeta],
    org: &'a RequirementOrganization,
    project: &'a str,
    labels: HashMap<String, String>,
}

impl OrganizationTreeBuilder<'_> {
    fn leaf(&mut self, doc: &RequirementDocumentMeta) -> TreeItem<'static, String> {
        let state = match doc.projects.get(self.project) {
            Some(RequirementProjectStatus::Implemented) => " · implemented",
            Some(RequirementProjectStatus::Verified) => " · verified",
            _ => "",
        };
        let id = format!("{TREE_DOC_PREFIX}{}", doc.id);
        let label = format!("{}{state}", requirement_document_tree_label(doc));
        self.labels.insert(id.clone(), label.clone());
        TreeItem::new_leaf(id, label)
    }

    fn node(
        &mut self,
        id: String,
        label: String,
        children: Vec<TreeItem<'static, String>>,
    ) -> TreeItem<'static, String> {
        self.labels.insert(id.clone(), label.clone());
        // Children are distinct collections plus distinct document IDs, so identifiers are unique.
        TreeItem::new(id, label, children).expect("unique requirement tree children")
    }

    /// Folders (and folder-nested queries) directly below `parent`.
    fn children(&mut self, parent: Option<&str>) -> Vec<TreeItem<'static, String>> {
        let org = self.org;
        org.collections
            .iter()
            // Root queries live in the Saved queries group instead of between folders.
            .filter(|c| {
                c.parent.as_deref() == parent && (parent.is_some() || c.condition.is_none())
            })
            .map(|c| self.collection(c))
            .collect()
    }

    fn collection(
        &mut self,
        c: &teshi_engine::organization::RequirementCollection,
    ) -> TreeItem<'static, String> {
        let (docs, org, project) = (self.docs, self.org, self.project);
        let members: Vec<_> = docs
            .iter()
            .filter(|d| match &c.condition {
                Some(q) => org.matches(q, d, project),
                None => c.documents.contains(&d.id),
            })
            .collect();
        let mut children = self.children(Some(&c.id));
        let count = members.len();
        children.extend(members.into_iter().map(|d| self.leaf(d)));
        let label = if c.condition.is_some() && c.parent.is_some() {
            format!("{} (query, {count})", c.name)
        } else {
            format!("{} ({count})", c.name)
        };
        self.node(format!("{TREE_COLLECTION_PREFIX}{}", c.id), label, children)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_input_edits_at_unicode_cursor() {
        let mut input = LineInput::new("需求");
        input.move_cursor(CursorMotion::Left);
        input.insert('a');
        assert_eq!(input.text(), "需a求");
        assert_eq!(input.split_at_cursor(), ("需a", "求"));
        input.move_cursor(CursorMotion::Home);
        input.backspace();
        input.delete();
        assert_eq!(input.text(), "a求", "backspace at the start is a no-op");
        input.move_cursor(CursorMotion::End);
        input.delete();
        input.backspace();
        assert_eq!(input.text(), "a");
    }
}
