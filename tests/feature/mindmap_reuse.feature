# language: en

@integration
Feature: Cross-file step reuse detection in MindMap
  # Medium granularity: focuses on StepIndex construction and the visualization of cross-file step
  #   reuse in the MindMap tree.
  # Stable contracts: `StepIndex::build` normalizes step text across features and scenarios,
  #   `MindMapIndex::build_index` builds a trie and merges identical step texts into a single node,
  #   `selected_node_context` returns a `MindMapContext` (with path_labels and location_count)

  Background:
    Given the project directory contains two feature files:
      | File               | Content                                                    |
      | login.feature      | Scenario: Login — Given I am on the login page             |
      |                    | Scenario: Logout — Given I am on the login page            |
      | dashboard.feature  | Scenario: View dashboard — Given I am on the login page    |
      And the TUI is running and has loaded the project

  Scenario: Identify cross-file reused steps in the MindMap tree
    When I press `2` to switch to the MindMap tab
      And I expand tree nodes until I see step-level leaf nodes
    Then the step "I am on the login page" should appear as a single node
      And that node should indicate a location_count >= 3 (or show it is reused in multiple places)
      And selecting that node should display a list of all occurrence paths
          (login.feature and dashboard.feature)

  Scenario: Cycle through locations of a reused step using Tab
    Given the MindMap tab is active and a cross-file reused step node is selected
    When I press `Tab`
    Then the preview panel on the right should jump to the next occurrence of that step
      And the preview title should update to show the corresponding feature file name and Scenario name
    When I press `Tab` multiple times
    Then it should cycle through all occurrences of that step and eventually return to the first one
