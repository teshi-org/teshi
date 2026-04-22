@editor
Feature: BDD Editor
  As a BDD practitioner
  I want to navigate and edit Gherkin feature files with structure-aware keybindings
  So that I can efficiently write and maintain test scenarios

  # --- Navigation ---

  Scenario: Navigate down between BDD nodes
    Given I have opened a feature file
    And the cursor is on a BDD node
    When I press the down arrow key
    Then the cursor moves to the next BDD node
    And non-structural lines are skipped

  Scenario: Navigate up between BDD nodes
    Given I have opened a feature file
    And the cursor is on a BDD node
    When I press the up arrow key
    Then the cursor moves to the previous BDD node

  Scenario: Toggle between keyword focus and body focus
    Given the cursor is on a step line
    And the focus is on keyword
    When I press the right arrow key
    Then the focus switches to body

  Scenario: Toggle between body focus and keyword focus
    Given the cursor is on a step line
    And the focus is on body
    When I press the left arrow key
    Then the focus switches to keyword

  Scenario: Body-chain vertical navigation
    Given the focus is on body
    And the cursor is on a step or editable title line
    When I press the down arrow key
    Then the cursor moves to the next step or editable title line
    And Scenario and Feature title lines are included in the chain

  Scenario: Jump to first BDD node
    Given I have opened a feature file
    When I press the Home key
    Then the cursor moves to the first BDD node

  Scenario: Jump to last BDD node
    Given I have opened a feature file
    When I press the End key
    Then the cursor moves to the last BDD node

  Scenario: Page navigation
    Given I have opened a feature file with many nodes
    When I press PageDown
    Then the cursor advances by approximately 10 BDD nodes
    When I press PageUp
    Then the cursor retreats by approximately 10 BDD nodes

  # --- Step body editing ---

  Scenario: Activate step body editing
    Given the cursor is on a step line
    And the focus is on body
    When I press Space
    Then step input mode activates
    And the cursor moves to the end of the line

  Scenario: Type characters in step input mode
    Given step input mode is active
    When I type a printable character
    Then the character is inserted at the cursor position

  Scenario: Commit step body edit
    Given step input mode is active
    When I press Enter
    Then step input mode deactivates
    And the edit is preserved in the buffer

  Scenario: Backspace respects keyword boundary
    Given step input mode is active
    And the cursor is at the body start position
    When I press Backspace
    Then no character is deleted
    And the cursor does not move

  Scenario: Delete key in step input mode
    Given step input mode is active
    And the cursor is not at the end of the line
    When I press Delete
    Then the character after the cursor is removed

  Scenario: Cancel editing with Escape
    Given step input mode is active
    When I press Escape
    Then step input mode deactivates

  # --- Step keyword picker ---

  Scenario: Open step keyword picker
    Given the cursor is on a step line
    And the focus is on keyword
    When I press Space
    Then the step keyword picker opens
    And the current keyword is pre-selected

  Scenario: Navigate keyword picker
    Given the step keyword picker is open
    When I press the down arrow key
    Then the selection moves to the next keyword option

  Scenario: Confirm keyword selection
    Given the step keyword picker is open
    And a different keyword is highlighted
    When I press Enter
    Then the step keyword is replaced in the buffer
    And the picker closes

  Scenario: Cancel keyword selection
    Given the step keyword picker is open
    When I press Escape
    Then the picker closes
    And the original keyword is preserved

  Scenario: Keyword picker unavailable on header lines
    Given the cursor is on a Feature header line
    And the focus is on keyword
    When I press Space
    Then no picker opens
    And a status message indicates step lines only

  # --- Header and description editing ---

  Scenario: Edit Feature title
    Given the cursor is on a Feature header line
    And the focus is on body
    When I press Space
    Then step input mode activates on the title text after the colon

  Scenario: Edit Scenario title
    Given the cursor is on a Scenario header line
    And the focus is on body
    When I press Space
    Then step input mode activates on the title text after the colon

  Scenario: Edit feature description lines
    Given the cursor is on a feature narrative line below the Feature header
    When I press Space
    Then step input mode activates
    And editing starts from column 0 of the line

  # --- Syntax highlighting ---

  Scenario: Gherkin keyword highlighting
    Given a feature file is loaded
    Then structural keywords are rendered in distinct colors
    And step keywords are rendered in distinct colors

  Scenario: Tag highlighting
    Given a line starts with a tag like "@smoke"
    Then the tag is rendered in the tag color

  Scenario: String highlighting
    Given a step contains a quoted string like "alice"
    Then the quoted string is rendered in the string color

  Scenario: Table and doc string highlighting
    Given a feature file contains table rows starting with "|"
    Then the table delimiters are highlighted
    And doc string markers are highlighted

  # --- File operations ---

  Scenario: Save file
    Given the buffer has been modified
    When I press s
    Then the file is written to disk
    And the status bar shows the saved path
    And the dirty flag is cleared

  Scenario: Quit with unsaved changes requires confirmation
    Given the buffer has been modified
    When I press q
    Then a confirmation message is shown in the status bar
    When I press q again
    Then the application exits

  Scenario: Quit without unsaved changes
    Given the buffer has not been modified
    When I press q
    Then the application exits immediately

  # --- Tab switching ---

  Scenario: Switch between tabs
    Given the MindMap tab is active
    When I press 3
    Then the Help tab becomes active
    When I press 2
    Then the MindMap tab becomes active again

  Scenario: Tab switch clears active edit state
    Given step input mode is active in the editor panel
    When I switch to the Help tab
    Then step input mode is deactivated
    And any open keyword picker is closed
