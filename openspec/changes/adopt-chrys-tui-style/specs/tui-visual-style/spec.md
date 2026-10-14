## ADDED Requirements

### Requirement: Theme module is the single source of TUI colors
The `teshi-tui` crate SHALL define every color and reusable component style in `crates/teshi-tui/src/theme.rs`. Page and widget code MUST obtain colors through named theme tokens or theme helper functions and MUST NOT construct `ratatui::style::Color` values directly. A unit test SHALL enforce a per-file cap on raw `Color::` occurrences outside `theme.rs`; caps MUST only decrease, and every cap MUST be zero once the migration completes.

#### Scenario: New raw color fails the guard
- **WHEN** a contributor adds a `Color::Rgb(...)` or named `Color::` value to a TUI page file
- **THEN** the color-budget test fails and names the file whose count exceeds its cap

#### Scenario: Migration completes with zero raw colors
- **WHEN** all migration phases are finished
- **THEN** the color-budget test passes with a cap of zero for every source file except `theme.rs`

### Requirement: Terminal background is never painted
The TUI SHALL leave the terminal's own background visible across the full screen. Background fills MUST be limited to small surfaces defined by the theme: selected rows, chips, the active tab, the footer strip, code and quote blocks, editor selection and current line, overlays, and diff tints.

#### Scenario: Empty panel area keeps the terminal background
- **WHEN** a tab renders with unused space inside its panels
- **THEN** those cells carry no background color from the TUI

### Requirement: Quiet chrome with focus emphasis
Every bordered panel and popup SHALL use rounded borders. An unfocused panel border MUST use the theme `BORDER` token, and the focused panel border MUST use the theme `PRIMARY` token. Panel titles MUST render in `PRIMARY` with bold weight.

#### Scenario: Focus moves between Explore columns
- **WHEN** focus moves from the Features column to the Scenarios column
- **THEN** the Scenarios border switches to `PRIMARY` and the Features border switches to `BORDER`, both drawn with rounded corners

### Requirement: Solid-fill selection
Selected rows in lists, trees, columns, and option pickers SHALL fill the entire row width with a background color instead of changing only the foreground. A selection in the focused widget MUST use the theme cursor background and cursor foreground tokens, and a selection in an unfocused widget MUST use the inactive selection background with the default foreground.

#### Scenario: Selected feature in the focused column
- **WHEN** the Features column is focused and a feature is selected
- **THEN** that row is filled edge to edge with the cursor background and its text uses the cursor foreground

#### Scenario: Selection persists after focus leaves
- **WHEN** focus moves away from a column that has a selected row
- **THEN** the row keeps a full-width fill using the inactive selection background

### Requirement: Tab bar with active underline
The top tab bar SHALL render inactive tabs in the inactive tab color and the active tab in bold on the active tab background. The row directly below the tab bar SHALL show a thin `─` rule in `BORDER` with a `━` segment in `PRIMARY` spanning exactly the active tab's columns, and the version label MUST stay right-aligned on that row in the muted color. Tab titles, padding, divider, and click regions MUST stay unchanged, so mouse hit-testing still selects the tab under the cursor.

#### Scenario: Underline follows the active tab
- **WHEN** the user switches from the Explore tab to the MindMap tab
- **THEN** the `━` segment moves under the MindMap tab title and the Explore tab returns to the inactive style

#### Scenario: Clicking a tab still activates it
- **WHEN** the user clicks inside any tab title's region
- **THEN** that tab becomes active, exactly as before the restyle

### Requirement: Footer key hints
Every footer key-hint line SHALL render each key in `PRIMARY` bold and its description in the footer description color on the footer background strip. The hint text and key bindings MUST stay unchanged. A transient status message MUST render on the same row in `WARNING` bold.

#### Scenario: Explore footer hints
- **WHEN** the Explore tab footer is visible
- **THEN** each hint shows its key in `PRIMARY` bold followed by its description, with the same wording as before

### Requirement: Consistent popups
Every popup and overlay SHALL clear its area, draw a rounded `PRIMARY` border with a `PRIMARY` bold title, highlight the current option with the solid-fill selection style, and render confirm/cancel actions and mode badges as chips (a solid background with dark foreground).

#### Scenario: Quit confirmation popup
- **WHEN** the quit confirmation popup opens
- **THEN** it shows a rounded `PRIMARY` border, a bold `PRIMARY` title, and its actions as chips

### Requirement: Semantic status and diff colors
Run status, git status, and change diffs SHALL use semantic theme roles. Pending MUST use the muted color, running `WARNING`, passed `SUCCESS`, failed `ERROR`, and skipped `BORDER`. Added, modified, and deleted lines MUST use `SUCCESS`, `WARNING`, and `ERROR` foregrounds, and when a row is tinted it MUST use the matching added/modified/deleted diff background token.

#### Scenario: Failed scenario run
- **WHEN** a scenario run fails
- **THEN** its status marker in the Explore tab renders in `ERROR`

#### Scenario: Deleted git line in the Steps column
- **WHEN** the Steps column shows a deleted line from the git diff
- **THEN** the line uses the `ERROR` foreground and, if tinted, the deleted diff background

### Requirement: Gherkin highlighting uses the chrys palette
Gherkin syntax highlighting in the editor, the Explore Steps column, and the MindMap preview SHALL share one mapping:
- Header: default foreground, bold
- Tag: `ACCENT`
- Given: `PRIMARY`
- When: `WARNING`
- Then: `SUCCESS`
- And/But: the inactive tab color
- Comment: muted
- String: the syntax-string token
- Meta and DocString: `SECONDARY`

The existing And/But inheritance behavior MUST be preserved.

#### Scenario: Step keywords in the editor
- **WHEN** a feature file with Given, When, and Then steps opens in the editor
- **THEN** the keywords render in `PRIMARY`, `WARNING`, and `SUCCESS` respectively

#### Scenario: Explore and editor agree
- **WHEN** the same step appears in the Explore Steps column and in the editor
- **THEN** its keyword uses the same color in both places

### Requirement: Fixed glyph vocabulary
TUI surfaces SHALL use only the glyphs listed in the style guide for status (`● ○ ◆ ✓ ✗`), spinners (`◐◓◑◒`), rails (`▌`), scrollbars (track `│`, thumb `┃`), bullets (`• ▪ ‣ ⭑ ◦`), menu current item (`◦`), and checkboxes (`[*] [ ]`). New glyphs MUST be added to the style guide before use.

#### Scenario: Scrollbar glyphs
- **WHEN** any scrollable column or panel overflows
- **THEN** its scrollbar draws the track with `│` and the thumb with `┃` in the scrollbar theme colors

### Requirement: Documented style guide
The repository SHALL include `doc/tui-style-guide.md`, in English, that documents the design rules, token table, component helpers, glyph vocabulary, do/don't examples, and a migration checklist. It SHALL also include a Cursor rule scoped to `crates/teshi-tui/**/*.rs` that directs contributors to the guide and the theme module.

#### Scenario: Contributor adds a new panel
- **WHEN** a contributor adds a new TUI panel
- **THEN** the guide tells them which theme helper to use for the block, selection, and footer hints, and the Cursor rule surfaces that guidance while they edit TUI files
