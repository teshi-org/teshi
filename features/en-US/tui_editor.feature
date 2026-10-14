# language: en

@tui
Feature: Terminal feature editing
  Edit and save a feature while navigating the terminal application.

  Background:
    Given Teshi is open on a sample project

  Scenario: Navigate between views and exit
    Given the sample project is visible
    When I open AI chat
    Then the chat introduction is visible
    When I return to Explore
    Then the sample scenario is visible
    When I quit Teshi
    Then Teshi exits successfully

  Scenario: Run, edit, and save a feature
    Given the sample project is visible
    When I run the sample scenario
    And I open the sample editor
    Then the sample test has passed
    When I edit the sample step
    Then the edited step is visible
    When I save the feature
    Then the edited step remains visible
    When I quit Teshi
    Then Teshi exits successfully
