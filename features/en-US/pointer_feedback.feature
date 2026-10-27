# language: en
# Requirement: doc-35; partially bound; see test results for scenario status.
@tui
Feature: Pointer feedback

  Scenario: Highlight a clickable target
    Given a visible action button
    When I move the pointer onto it and away
    Then the target gains and loses a consistent visible hover style

  Scenario: Modal coverage blocks underlying feedback
    Given a modal covering a page action
    When I move the pointer over the covered action
    Then the covered action is not shown as available
