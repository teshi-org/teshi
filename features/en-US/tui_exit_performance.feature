# language: en
# Requirement source: conversation about slow quit and Chrys shutdown behavior.
# Timing is measured from the quit key to actual process exit in a real PTY.

@tui
Feature: TUI exit performance
  Quit promptly while preserving conversations and cleaning up owned agent processes.

  Background:
    Given Teshi is open with an isolated project and conversation storage
    And the project has no unsaved edits
    And keyboard focus allows "Ctrl+Q" to quit Teshi

  Scenario Outline: Quit promptly when no agent work is pending
    Given the agent state is "<state>"
    When I press "Ctrl+Q"
    Then the Teshi process exits successfully within 1000 milliseconds
    And the terminal input mode and screen are restored

    Examples:
      | state                                  |
      | Native with no active turn             |
      | ACP selected but not connected         |
      | ACP idle after a completed reply       |

  Scenario Outline: Interrupt pending ACP work when quitting
    Given a controlled ACP agent is waiting at "<phase>"
    And the agent exits promptly when its input is closed
    When I press "Ctrl+Q"
    Then the Teshi process exits successfully within 3000 milliseconds
    And no process owned by that ACP agent remains running
    And the terminal input mode and screen are restored

    Examples:
      | phase                    |
      | initialization           |
      | authentication           |
      | session creation         |
      | streaming a reply        |
      | waiting for permission   |

  Scenario Outline: Bound shutdown when an ACP agent does not cooperate
    Given a controlled ACP agent "<behavior>"
    When I press "Ctrl+Q"
    Then the Teshi process exits successfully within 5000 milliseconds
    And no process owned by that ACP agent remains running
    And the terminal input mode and screen are restored

    Examples:
      | behavior                                             |
      | ignores cancellation and input closure               |
      | leaves a child process running after its own exit    |

  Scenario: Close multiple idle ACP conversations without serial delays
    Given 3 conversations each have a connected controlled ACP agent
    And all 3 agents have completed their replies and exit promptly on input closure
    When I press "Ctrl+Q"
    Then the Teshi process exits successfully within 1000 milliseconds
    And no process owned by those ACP agents remains running

  Scenario: Preserve a completed conversation across a prompt exit
    Given the selected backend is "ACP" with a controlled agent
    And the agent has completed its reply "Inspection finished"
    When I press "Ctrl+Q"
    Then the Teshi process exits successfully within 1000 milliseconds
    When I reopen Teshi with the same project and conversation storage
    And I open AI chat
    Then "Inspection finished" is visible in AI chat
