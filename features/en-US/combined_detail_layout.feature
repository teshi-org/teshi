# language: en
# Requirement: doc-36; partially bound; see test results for scenario status.
@tui
Feature: Combined detail layout

  Scenario: Combine supporting details in two columns
    Given a normal width Requirements, Testpoints or Explore tab
    When I view its navigation and details
    Then navigation is on the left and the two detail panes share the right column vertically

  Scenario: Keep all panes reachable on a small screen
    Given a compact tab shown in a narrow terminal
    When I cycle keyboard focus through every pane
    Then each pane is shown when focused with its existing selection
