# language: en

@web-ui
Feature: Run and inspect scenarios
  As a tester using teshi web
  I want to run Gherkin scenarios and inspect their events
  So that I can see step results without editing feature files in the GUI

  Background:
    Given teshi web is running at http://127.0.0.1:20253/?e2e=1
    And teshi web has a project with Gherkin scenarios open
    And the Browser Profiles surface is shown

  Scenario: User opens Run from the shell header
    When the user opens the Run surface
    Then the Run inspect surface is shown
    And teshi web does not show the settings form

  Scenario: Run lists Gherkin scenarios from the open project
    Given the Run inspect surface is shown
    When the user views the scenario list
    Then teshi web lists Gherkin scenarios from the open project

  Scenario: Refresh reloads the scenario list
    Given the Run inspect surface is shown
    When the user refreshes the scenario list
    Then teshi web lists Gherkin scenarios from the open project

  Scenario: Running a listed scenario shows an event log
    Given the Run inspect surface is shown
    And a Gherkin scenario is selected
    When the user starts that scenario
    Then an event log is shown for the run

  Scenario: Run inspect surface does not edit Gherkin files
    Given the Run inspect surface is shown
    When the user inspects the run events
    Then teshi web does not offer a Gherkin editor

  Scenario: User can expand redacted secrets after a run
    Given a completed run with redacted HTTP values is shown
    When the user expands secrets
    Then teshi web reveals the previously redacted values

  Scenario: User returns to Browser Profiles from Run
    Given the Run inspect surface is shown
    When the user opens the Browser Profiles surface
    Then the Browser Profiles surface is shown
