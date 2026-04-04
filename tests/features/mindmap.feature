@mindmap
Feature: Mind Map Tree View
  As a BDD practitioner managing multiple feature files
  I want to see all features in a collapsible tree with keyword-free step nodes
  So that I can discover step reuse across features and navigate the project structure

  # --- Tree structure ---

  Scenario: Display project tree hierarchy
    Given I have opened a directory containing feature files
    Then the MindMap tab shows a collapsible tree
    And the root node represents the project directory
    And each scenario's step sequence appears as a path under the root
    And shared step prefixes are merged into a single path
    And no feature file nodes are shown in the tree

  Scenario: Step nodes display body text without keywords
    Given a scenario contains the step "Given I am on the login page"
    Then the tree displays the step node as "I am on the login page"
    And no Given, When, Then, And, or But keyword is shown

  Scenario: Tags are not shown in the tree
    Given a feature file has tags "@auth @smoke" above the Feature line
    Then no tag node appears in the tree
    And no tag text appears in the tree

  Scenario: Examples table is not shown in the tree
    Given a Scenario Outline has an Examples table
    Then no Examples node appears in the tree
    And no table row nodes appear in the tree

  Scenario: Background steps are included as shared prefixes
    Given a feature contains a Background with steps
    And multiple scenarios follow that Background
    Then the tree paths start with the Background steps
    And the Background steps appear as a shared prefix across those scenarios

  # --- Step reuse detection ---

  Scenario: Identical step prefixes across files are merged
    Given Feature A contains "When I am on the login page"
    And Feature B contains "Given I am on the login page"
    Then the tree displays a shared step node "I am on the login page"
    And the path to that node is shared when prefixes match

  Scenario: Shared step does not show reuse suffix
    Given the step body "I am on the login page" appears in 3 scenarios
    Then the tree does not show any reuse suffix like "[x3]"

  Scenario: Unique steps have no reuse annotation
    Given a step body appears in only one scenario
    Then the tree node has no reuse suffix

  Scenario: Shared path offers multiple preview locations
    Given a shared step path exists in multiple scenarios
    Then a location strip shows "Location 1/N" above the preview
    When I press ]
    Then the preview switches to another Feature and Scenario occurrence
    When I press [
    Then the preview switches to the previous occurrence

  # --- Tree navigation ---

  Scenario: Move selection down in tree
    Given the MindMap tree is displayed
    When I press the down arrow key
    Then the selection moves to the next visible tree node

  Scenario: Move selection up in tree
    Given the MindMap tree is displayed
    When I press the up arrow key
    Then the selection moves to the previous visible tree node

  Scenario: Collapse a tree node
    Given a tree node is expanded and has children
    When I press the left arrow key
    Then the children are hidden
    And the node shows a collapsed indicator

  Scenario: Expand a tree node
    Given a tree node is collapsed and has children
    When I press the right arrow key
    Then the children become visible
    And the node shows an expanded indicator

  Scenario: Tree is read-only
    Given the selection is on a step node in the tree
    When I press Space
    Then no editing mode activates
    And the tree content remains unchanged

  # --- Three-stage view transitions ---

  Scenario: Stage 1 - tree occupies full width
    Given I have opened a directory of feature files
    Then the tree panel occupies the full terminal width
    And no editor or reserved panel is visible

  Scenario: Stage 1 to Stage 2 - open editor preview
    Given the view is in Stage 1
    When I press Enter on a tree node
    Then the view transitions to Stage 2
    And the tree panel shrinks to approximately 45 percent width
    And the editor preview panel appears on the right at approximately 55 percent width
    And the corresponding feature file content is shown in the editor preview

  Scenario: Stage 2 - editor preview tracks tree selection
    Given the view is in Stage 2
    When I move the selection to a different tree node
    Then the editor preview scrolls to the line corresponding to the selected node
    And the selected line is highlighted in the editor preview

  Scenario: Stage 2 - cross-file navigation auto-switches buffer
    Given the view is in Stage 2
    And the editor preview shows editor.feature
    When I navigate to a node belonging to mindmap.feature
    Then the editor preview automatically switches to mindmap.feature
    And the view scrolls to the corresponding line

  Scenario: Stage 2 - editor preview shows full Gherkin with keywords
    Given the view is in Stage 2
    And a step in the tree shows "I am on the login page"
    Then the editor preview shows the full line "Given I am on the login page"
    And the Gherkin syntax highlighting is applied

  Scenario: Stage 2 to Stage 3 - enter editor with reserved panel
    Given the view is in Stage 2
    When I press the right arrow key on a leaf node with no children
    Then the view transitions to Stage 3
    And the tree panel is completely hidden
    And the editor panel moves to the left at approximately 65 percent width
    And the reserved panel appears on the right at approximately 35 percent width

  Scenario: Stage 2 to Stage 3 - cursor lands on selected node line
    Given the view is in Stage 2
    And the selected tree node corresponds to line 10 of the feature file
    When I press the right arrow key to enter Stage 3
    Then the editor cursor is positioned at line 10
    And the focus is on keyword

  Scenario: Stage 3 - full editor functionality
    Given the view is in Stage 3
    Then all BDD navigation features are available in the editor panel
    And step body editing via Space is available
    And step keyword picker via Space on keyword focus is available
    And save and quit keybindings work normally

  Scenario: Stage 3 - reserved panel shows placeholder
    Given the view is in Stage 3
    Then the reserved panel displays a placeholder message
    And the placeholder indicates planned features including step impl code
    And the placeholder indicates planned features including BDD executor
    And the placeholder indicates planned features including test results

  Scenario: Stage 3 to Stage 2 - return to tree
    Given the view is in Stage 3
    And no edit mode is active
    And the focus is on keyword
    When I press the left arrow key
    Then the view transitions to Stage 2
    And the tree panel reappears on the left
    And the reserved panel is hidden

  Scenario: Stage 3 to Stage 2 - tree selection syncs to editor position
    Given the view is in Stage 3
    And I have navigated from "Scenario: Login" to "Scenario: Search" in the editor
    When I press the left arrow key to return to Stage 2
    Then the tree selection updates to the node closest to the editor cursor position

  Scenario: Stage 2 to Stage 1 - close editor preview
    Given the view is in Stage 2
    When I press Escape
    Then the view transitions to Stage 1
    And the editor preview panel is hidden
    And the tree takes the full terminal width
