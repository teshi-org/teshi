# language: en

@e2e
Feature: Launch and browse a BDD project from the terminal
  # Coarse granularity: covers the full user journey of starting the TUI → loading .feature files →
  #   three-column navigation → switching tabs.
  # Stable contracts: CLI `teshi <dir>`, `App::from_directory`, Explore three-column rendering
  #   (ui.rs render_explore_panel), `MainTab` tab switching (`handle_action SelectTab`),
  #   keymap (keymap.rs Action::MoveUp/MoveDown/TreeUp, etc.)

  Background:
    Given the current working directory contains at least two valid .feature files
      And one file contains three Scenarios and the other contains one Scenario

  Scenario: Open a directory and browse the feature list
    When I run `teshi .` in the terminal
    Then the TUI should start and display the Explore tab
      And the left "Features" column should list all .feature file names
      And the first file should be highlighted by default

  Scenario: Navigate between the Scenario and Step columns using the keyboard
    Given the TUI is running on the Explore tab
      And a feature file is selected in the left Features column
    When I press `→` or `Tab`
    Then focus should move to the "Scenarios" column
      And that column should list all Scenario names from the selected feature file
    When I press `→` or `Tab` again
    Then focus should move to the "Steps" column
      And that column should list all step texts from the selected Scenario
    When I press `←` or `BackTab`
    Then focus should return to the previous column

  Scenario: Switch to the MindMap tab and view the step tree
    Given the TUI is running on the Explore tab
    When I press the `2` key
    Then the active tab should switch to "MindMap"
      And the left side should display a collapsible tree with the project directory name as the root
      And expanded nodes should show feature file names as first-level children
      And expanding a feature node should show Scenario names and all their steps

  Scenario: Switch to the Help tab and view the keybinding reference
    Given the TUI is running
    When I press the `4` key
    Then the active tab should switch to "Help"
      And the content area should display keybinding reference text with sections such as "Tabs", "MindMap", and "Explore"
