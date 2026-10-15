# language: en
# Requirement source: reqstore-18d3e4036ad71b3800002674/doc-6

@tui
Feature: Agent execution activity
  Show agent reasoning and tool progress in AI chat while a turn is running.

  Background:
    Given Teshi is open with an isolated project and conversation storage
    And I open AI chat

  Scenario Outline: Show reasoning before the final answer
    Given the selected backend is "<backend>" with a controlled agent
    When I send a message requesting a project inspection
    And the agent streams reasoning "Inspecting the project" and waits before answering
    Then "Inspecting the project" is visible in AI chat
    And the final answer is not yet visible
    When the agent completes with the answer "Inspection finished"
    Then "Inspection finished" is visible in AI chat
    And the reasoning "Inspecting the project" appears exactly once in the conversation

    Examples:
      | backend |
      | Native  |
      | ACP     |

  Scenario Outline: Keep the title when an ACP tool update omits it
    Given the selected backend is "ACP" with a controlled agent
    When I send a message requesting a project inspection
    And the agent starts tool call "read-project" titled "Read project" with status "in_progress"
    Then AI chat shows tool "Read project" with status "in_progress"
    When the agent updates tool call "read-project" to "<status>" without a title
    Then AI chat shows tool "Read project" with status "<status>"
    And the conversation contains exactly one entry for tool call "read-project"
    When the agent finishes the turn
    Then AI chat still shows tool "Read project" with status "<status>"

    Examples:
      | status    |
      | completed |
      | failed    |

  Scenario: Show native tool requests and results
    Given the selected backend is "Native" with a controlled agent
    When I send a message requesting a project inspection
    And the agent requests a project inspection tool
    Then the tool request is visible in AI chat
    When the tool returns "Project inspection complete"
    Then the tool result "Project inspection complete" is visible in AI chat
    When the agent finishes the turn
    Then the tool request and its result remain visible in the conversation
