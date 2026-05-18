# language: en

@integration
Feature: Edit Gherkin steps in Editor mode
  # Medium granularity: focuses on single editor-mode operations — entering edit, selecting a
  #   keyword, inserting/deleting steps, and undoing.
  # Stable contracts: `explore_enter_edit`, `begin_step_or_title_edit`, `step_keyword_picker`,
  #   `insert_step` (Tab key), `delete_current_node` (dd), `undo` (Ctrl+/),
  #   `bdd_nav::step_edit_start_col`, `keymap::Action::SwitchKeyword`

  Background:
    Given the TUI is running and has loaded a feature file with the following content:
      """
      Feature: User Login
        Scenario: Successful login
          Given I am on the login page
          When I enter my username
      """
      And the Explore tab is active with the cursor on the "Given I am on the login page" step
          in the "Successful login" Scenario

  Scenario: Enter step edit mode and modify step text
    When I press `e` to enter the editor
      And the cursor is on the "Given I am on the login page" line
      And I press `Space` (with focus on the body area)
    Then the line should enter step input mode (step_input_active)
      And I should be able to append or modify text at the end of the line
    When I press `Esc`
    Then step input mode should exit, returning to navigation mode

  Scenario: Switch the step keyword using the keyword picker
    Given I have entered the editor with the cursor on the "Given I am on the login page" line
      And focus is on the keyword slot (focus_slot = Keyword)
    When I press `Space`
    Then the step keyword picker should appear (step_keyword_picker)
      And the picker should contain the options "Given", "When", "Then", "And", and "But"
    When I use `↑`/`↓` to move to "When" and press `Enter`
    Then the line's keyword should change to "When"
      And the keyword picker should close

  Scenario: Insert a new step below the current one using Tab
    Given I have entered the editor with the cursor on the "Given I am on the login page" line
    When I press `Tab`
    Then a new empty step line with a default keyword should appear below the current line
      And the new step's indentation should match the current step

  Scenario: Delete the current step
    Given I have entered the editor with the cursor on the "Given I am on the login page" line
    When I press `d` then `d` in sequence
    Then that step line should be deleted
      And the remaining steps in the Scenario should shift up
    When I press `Ctrl+/` (undo)
    Then the deleted step line should be restored

  Scenario: Save changes and quit
    Given I have entered the editor and modified the step text
    When I press `s`
    Then the bottom status bar should briefly show a save confirmation message
      And the feature file on disk should contain the modified step text
    When I press `q`
    Then a confirmation prompt should appear (because the buffer is marked dirty)
    When I press `q` again
    Then the TUI should exit
