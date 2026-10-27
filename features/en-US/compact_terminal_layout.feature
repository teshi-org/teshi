# language: en
# Requirement: doc-31; partially bound; see test results for scenario status.
@tui
Feature: Compact terminal layout

  Scenario: Keep focused content accessible
    Given Requirements is open in a narrow terminal
    When I move focus across its panes
    Then each focused pane becomes visible without losing edits

  Scenario: Resize without stale pointer targets
    Given a terminal displaying multiple panes
    When I shrink and expand the terminal
    Then only visible controls respond to clicks and selection is preserved
