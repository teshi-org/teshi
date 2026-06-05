# language: en

@web-ui @embedded
Feature: Gherkin step selection in teshi web
  Requires `teshi web --project .` and a feature file selected in the tree.

  Background:
    Given teshi web is open with the current project

  Scenario: Select a Gherkin step from the welcome smoke feature
    When I open the welcome smoke feature in the file tree
    And I select the Open Project assertion step
    Then the Gherkin step should be highlighted
