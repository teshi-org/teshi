# Project requirement organization

The Requirements tree shows project-local views over shared requirement documents.
Folders and saved queries reference document IDs; neither moves nor copies Markdown.
Documents can appear in several folders and queries. Parents do not aggregate
descendant documents; the count after each name is its direct requirements.

In Current project scope the default **Folders** view lists **Inbox** first, then manual
folders, then a **Saved queries** group. New requirements appear in Inbox until filed in
a manual folder. Appearing in a query does not remove a requirement from Inbox; removing
its last folder returns it there. **g** switches to the Iteration view and back.

The tree title always lists the active settings in key order, for example
`Current project · Folders · Iteration: All · Pending only`. Implemented and verified
requirements are hidden by default; **H** shows them with state labels. Delivery state is
recorded per project, so **H** only applies in Current project scope. It filters the tree
and never changes stored state.

## TUI controls

- **Space**: open the action menu for the selected row. It lists only operations valid
  for that row, with their direct shortcut where one exists:
  - Requirement: Open in editor, Folders…, Link / unlink project (`b`),
    Set iteration… (`I`), Set priority… (`p`), Delete requirement… (`d`).
  - Inbox: New folder…, New requirement.
  - Folder: New subfolder…, New requirement here, Rename…, Delete folder….
  - Saved queries: New saved query….
  - Saved query: Edit query…, Rename…, Delete query….
- **Folders…** shows every folder as a checklist. Space toggles, Enter saves the exact
  set; no checked folder returns the requirement to Inbox.
- **New folder…** creates the name below the selected folder (or at the root from
  Inbox). `Chat/Archive` creates nested folders.
- **New requirement here** (or Ctrl+N with a folder selected) files the draft in that
  folder when it is first saved.
- **Delete folder…** also removes subfolders and reports how many requirements return
  to Inbox. A folder still used by a saved query cannot be deleted.
- **Enter**, `h`/`l` (or Left/Right), or a mouse click expands or collapses a folder,
  query, or iteration row; Enter on a requirement opens it in the editor. Repeated
  `l` steps into the first child and repeated `h` climbs to the parent, so the keys
  walk the tree like the MindMap pane; panes change with Tab/BackTab or Ctrl+W.

Text fields support Left/Right, Home/End, Backspace, and Delete. In the query form,
Tab or Up/Down switches between Name and Condition, and the form shows how many visible
requirements match while you type. Invalid input keeps the form open.

Organization edits require Current project scope. Query input supports `status`,
`priority`, `iteration`, `title` (case-insensitive substring), and `folder` (a folder
path such as `AI/Chat`, stored as its stable ID). Use `&` for AND, `|` for OR, and `!`
for negation. AND binds more tightly than OR. Parentheses and escaping separators are
not supported in text expressions; use the structured CLI JSON format for arbitrary
nesting or literal separator characters. Such queries cannot be edited in the TUI form.

## CLI

```powershell
teshi requirements organize --project . --folder "AI/Chat"
teshi requirements organize --project . --assign doc-14 doc-53 --to "AI/Chat"
teshi requirements organize --project . --query "Pending P1" --condition "status=planned & priority=P1"
teshi requirements organize --project . --query "Chat work" --condition "folder=AI/Chat & status=planned"
teshi requirements organize --project .
```

The read command returns JSON containing stable collection IDs, names, parent IDs,
manual document IDs, and optional conditions. `--assign` adds memberships without
removing existing ones. Referenced documents must be linked to this project first.

Use `--file organization.json` to replace the complete organization, including rename,
move, removal, exact membership edits, and nested query conditions. Export the current
configuration first to preserve IDs and memberships. For example:

```json
{
  "collections": [
    {"id": "ai", "name": "AI", "documents": ["doc-14"]},
    {"id": "pending", "name": "Pending AI", "condition": {
      "all": [{"folder": "ai"}, {"not": {"status": "verified"}}]
    }}
  ]
}
```

Conditions also support `any`, `priority`, `iteration`, and `title_contains`.
Query collections cannot own manual members or children. Cycles, invalid parents,
duplicate sibling names, and invalid conditions fail without writing configuration.

Organization is persisted under `<requirements-root>/views/<project-id>.json`, so
project worktrees sharing a durable ID and requirement store share their views.
The TUI polls external organization edits once per second and preserves dirty Markdown.
Concurrent incremental edits detect stale snapshots; full-file replacement is explicit.
Unlinked or deleted documents are omitted from views, while old references remain inert
until removed from the configuration.
