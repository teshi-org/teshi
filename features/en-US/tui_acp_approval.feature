# language: en

@tui
Feature: Change ACP agent approvals with a slash command
  Approval choices come from the connected agent and apply to its current session.

  Scenario: Confirm an advertised approval policy
    Given Teshi is open with an ACP agent offering approval policies
    When I open AI chat and run the approval command
    Then the agent's approval choices are shown
    When I select automatic approval
    Then reopening approvals shows automatic approval as current
    When I quit the approval conversation
    Then Teshi exits successfully

  Scenario: Keep the confirmed policy after rejection
    Given Teshi is open with an ACP agent that rejects a policy change
    When I open AI chat and run the approval command
    And I select the rejected policy
    Then reopening approvals shows the rejection and the original policy
    When I quit the approval conversation
    Then Teshi exits successfully

  Scenario: Use an agent's legacy session modes
    Given Teshi is open with an ACP agent offering legacy approval modes
    When I open AI chat and run the approval command
    Then the agent's approval choices are shown
    When I select automatic approval
    Then reopening approvals shows automatic approval as current
    When I quit the approval conversation
    Then Teshi exits successfully

  Scenario: Explain unavailable approval settings
    Given Teshi is open with an ACP agent without approval settings or modes
    When I open AI chat and run the approval command
    Then the unavailable approval capability is explained
    When I quit the approval conversation
    Then Teshi exits successfully

  Scenario: Keep the local policy separate from remote approvals
    Given Teshi is open with an ACP agent offering approval policies
    When I open AI chat and run the local approval command
    And I select the local automatic policy
    Then the remote approval policy still asks before changes
    When I quit the approval conversation
    Then Teshi exits successfully
