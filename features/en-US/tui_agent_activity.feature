# language: en
# Requirement source: reqstore-18d3e4036ad71b3800002674/doc-6

@tui
Feature: Agent execution activity
  Show agent reasoning and tool progress in AI chat while a turn is running.

  # Additional requirements: conversation feedback on final-reply color and event order.

  Background:
    Given Teshi is open with an isolated project and conversation storage
    And I open AI chat

  Scenario: Show reasoning before the final answer (Native)
    Given the selected backend is "Native" with a controlled agent
    When I send a message requesting a project inspection
    And the agent streams reasoning "Inspecting the project" and waits before answering
    Then "Inspecting the project" is visible in AI chat
    And the final answer is not yet visible
    When the agent completes with the answer "Inspection finished"
    Then "Inspection finished" is visible in AI chat
    And the reasoning "Inspecting the project" appears exactly once in the conversation

  Scenario: Show reasoning before the final answer (ACP)
    Given the selected backend is "ACP" with a controlled agent
    When I send a message requesting a project inspection
    And the agent streams reasoning "Inspecting the project" and waits before answering
    Then "Inspecting the project" is visible in AI chat
    And the final answer is not yet visible
    When the agent completes with the answer "Inspection finished"
    Then "Inspection finished" is visible in AI chat
    And the reasoning "Inspecting the project" appears exactly once in the conversation

  Scenario: Keep the title when an ACP tool update omits it (completed)
    Given the selected backend is "ACP" with a controlled agent
    When I send a message requesting a project inspection
    And the agent starts tool call "read-project" titled "Read project" with status "in_progress"
    Then AI chat shows tool "Read project" with status "in_progress"
    When the agent updates tool call "read-project" to "completed" without a title
    Then AI chat shows tool "Read project" with status "completed"
    And the conversation contains exactly one entry for tool call "read-project"
    When the agent finishes the turn
    Then AI chat still shows tool "Read project" with status "completed"

  Scenario: Keep the title when an ACP tool update omits it (failed)
    Given the selected backend is "ACP" with a controlled agent
    When I send a message requesting a project inspection
    And the agent starts tool call "read-project" titled "Read project" with status "in_progress"
    Then AI chat shows tool "Read project" with status "in_progress"
    When the agent updates tool call "read-project" to "failed" without a title
    Then AI chat shows tool "Read project" with status "failed"
    And the conversation contains exactly one entry for tool call "read-project"
    When the agent finishes the turn
    Then AI chat still shows tool "Read project" with status "failed"

  Scenario: Show native tool requests and results
    Given the selected backend is "Native" with a controlled agent
    When I send a message requesting a project inspection
    And the agent requests a project inspection tool
    Then the tool request is visible in AI chat
    When the tool returns "Project inspection complete"
    Then the tool result "Project inspection complete" is visible in AI chat
    When the agent finishes the turn
    Then the tool request and its result remain visible in the conversation

  Scenario: Reserve green reply styling for the completed final answer (Native)
    Given the selected backend is "Native" with a controlled agent
    When I send a message requesting a project inspection
    And the agent streams reply "Inspection finished" without finishing
    Then "Inspection finished" is visible in AI chat
    And reply "Inspection finished" has neutral styling without a green final-answer header or rail
    When the agent finishes the turn normally
    Then reply "Inspection finished" has a green final-answer header and rail
    And reply "Inspection finished" appears exactly once in the conversation

  Scenario: Reserve green reply styling for the completed final answer (ACP)
    Given the selected backend is "ACP" with a controlled agent
    When I send a message requesting a project inspection
    And the agent streams reply "Inspection finished" without finishing
    Then "Inspection finished" is visible in AI chat
    And reply "Inspection finished" has neutral styling without a green final-answer header or rail
    When the agent finishes the turn normally
    Then reply "Inspection finished" has a green final-answer header and rail
    And reply "Inspection finished" appears exactly once in the conversation

  Scenario: Keep ACP progress and tools in event order
    Given the selected backend is "ACP" with a controlled agent
    When I send a message requesting a project inspection
    And the agent streams reply "I will inspect the project" without finishing
    And the agent starts tool call "read-project" titled "Read project" with status "in_progress"
    Then reply "I will inspect the project" appears before tool "Read project"
    And reply "I will inspect the project" has neutral styling without a green final-answer header or rail
    When the agent updates tool call "read-project" to "completed" without a title
    And the agent completes with the answer "Inspection finished"
    Then the conversation displays these entries in order:
      | entry                      |
      | I will inspect the project |
      | Read project               |
      | Inspection finished        |
    And the conversation contains exactly one entry for tool call "read-project"
    And reply "I will inspect the project" has neutral styling without a green final-answer header or rail
    And reply "Inspection finished" has a green final-answer header and rail

  Scenario: Preserve progress styling after an interrupted turn (Native, cancellation)
    Given the selected backend is "Native" with a controlled agent
    When I send a message requesting a project inspection
    And the agent streams reply "Partial result" without finishing
    And the turn ends through "cancellation"
    Then "Partial result" is visible in AI chat
    And reply "Partial result" has neutral styling without a green final-answer header or rail

  Scenario: Preserve progress styling after an interrupted turn (Native, failure)
    Given the selected backend is "Native" with a controlled agent
    When I send a message requesting a project inspection
    And the agent streams reply "Partial result" without finishing
    And the turn ends through "failure"
    Then "Partial result" is visible in AI chat
    And reply "Partial result" has neutral styling without a green final-answer header or rail

  Scenario: Preserve progress styling after an interrupted turn (ACP, cancellation)
    Given the selected backend is "ACP" with a controlled agent
    When I send a message requesting a project inspection
    And the agent streams reply "Partial result" without finishing
    And the turn ends through "cancellation"
    Then "Partial result" is visible in AI chat
    And reply "Partial result" has neutral styling without a green final-answer header or rail

  Scenario: Preserve progress styling after an interrupted turn (ACP, failure)
    Given the selected backend is "ACP" with a controlled agent
    When I send a message requesting a project inspection
    And the agent streams reply "Partial result" without finishing
    And the turn ends through "failure"
    Then "Partial result" is visible in AI chat
    And reply "Partial result" has neutral styling without a green final-answer header or rail

  Scenario: Do not promote ACP progress when no final answer follows a tool
    Given the selected backend is "ACP" with a controlled agent
    When I send a message requesting a project inspection
    And the agent streams reply "I will inspect the project" without finishing
    And the agent starts tool call "read-project" titled "Read project" with status "in_progress"
    And the agent updates tool call "read-project" to "completed" without a title
    And the agent finishes the turn normally without further reply text
    Then reply "I will inspect the project" appears before tool "Read project"
    And reply "I will inspect the project" has neutral styling without a green final-answer header or rail
    And no reply in this turn has a green final-answer header or rail
