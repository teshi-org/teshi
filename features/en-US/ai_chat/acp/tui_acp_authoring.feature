# language: en

@tui
Feature: ACP authoring generation
  Drive ACP feature generation through the live TUI review and save workflow.

  Background:
    Given Teshi is open with a deterministic ACP authoring agent

  Scenario: Generate a feature through human review and Save
    Given I open AI chat for ACP authoring
    When I send ACP authoring requirements
    Then the ACP authoring review is visible
    When I approve the proposed test point
    And I continue ACP generation
    Then ACP requests the generated Feature file
    When I accept the generated Feature file
    Then ACP requests the scenario insertion
    When I accept the scenario insertion
    Then the generated Feature is visible in the editor
    When I save the generated Feature
    Then the generated Feature is saved to disk
    When I leave ACP authoring input
    And I quit ACP authoring
    Then ACP authoring exits successfully
