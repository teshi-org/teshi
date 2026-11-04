# language: en
# Requirement: doc-32; authored only; not bound, executed or accepted.
@tui
Feature: Explore step changes

  Scenario: Browse added and removed Steps
    Given a Scenario with an added Step and a removed Step
    When I focus Step details and press j then k
    Then selection moves between individually marked changes including the removed Step

  Scenario: Bound navigation at the first change
    Given the first Step change is selected
    When I press k
    Then selection remains at the first change
