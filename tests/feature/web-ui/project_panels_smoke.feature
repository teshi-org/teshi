# language: en

@web-ui @embedded
Feature: teshi web project panels smoke
  Requires `teshi web --project .` so panels load with an open project.

  Background:
    Given teshi web is open with the current project

  Scenario: Gherkin panel is visible with a project loaded
    Then the Gherkin panel should be visible

  Scenario: File tree tab is visible
    Then the Files tab should be visible
