# language: en
# Requirement: doc-34; partially bound; see test results for scenario status.
@tui
Feature: Footer actions

  Scenario: Click a visible action
    Given a visible Requirements Save action
    When I click Save
    Then the same save behavior as the keyboard shortcut occurs

  Scenario: Clipped actions are not clickable
    Given a terminal too narrow to show all footer actions
    When I click unused footer space
    Then no hidden action is triggered
