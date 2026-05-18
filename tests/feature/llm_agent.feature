# language: en

@integration
Feature: LLM Agent assisted editing
  # Medium granularity: focuses on the full LLM tool-call flow — user sends a message → Agent
  #   invokes a tool → change is queued for confirmation → user accepts/rejects → applied to buffer.
  # Stable contracts: `agent::execute_tool` dispatch, `AgentPendingChange` queue,
  #   `queue_agent_change` / `accept_agent_change` / `reject_agent_change`,
  #   TUI agent_change_prompt rendering ("[Y] accept [N] reject [Esc] reject"),
  #   `agent::tools::get_tools` six-tool registration,
  #   `AgentThread` per-agent state, sidebar rendering with status icons

  Background:
    Given the environment variable TESHI_LLM_API_KEY is set to a valid API key
      And the TUI is running and has loaded a project containing a feature file
      And I have pressed `3` to switch to the AI tab

  Scenario: Send a message to the LLM and receive a text reply
    When I type "Tell me about the current project" in the AI input box
      And I press `Enter` to send the message
    Then my user message should appear in the chat history
      And the AI status should change to "Waiting"
      And after a brief delay, an Assistant reply message should be appended to the chat history
      And the reply should include basic information such as the project directory
          and the number of feature files

  Scenario: LLM requests to insert a new scenario via the insert_scenario tool
    Given there is an active conversation in the AI tab
    When I send the message "Please add a scenario named 'Password Reset' to login.feature
         with steps Given a logged-in user and When I click forgot password"
    Then the chat history should show that the LLM invoked the `insert_scenario` tool
      And the bottom status bar should display an "AI wants to modify a file" confirmation prompt
      And the prompt should include `[Y] accept`, `[N] reject`, and `[Esc] reject` options
    When I press `Y` to accept the change
    Then the bottom status bar should show a confirmation success message
      And the editor buffer for login.feature should contain the newly inserted
          "Scenario: Password Reset" block with its steps

  Scenario: User rejects an LLM file-modification request
    Given there is an active conversation in the AI tab
      And the LLM has just requested a step modification via the `update_step` tool
      And the bottom bar shows the "AI wants to modify a file" confirmation prompt
    When I press `N`
    Then the change should be discarded and the buffer should remain unchanged
      And the bottom status bar should show a rejection confirmation message
      And the LLM should receive tool result feedback indicating the change was rejected by the user

  Scenario: LLM calls highlight_mindmap_nodes to highlight MindMap nodes
    Given there is an active conversation in the AI tab
    When I send the message "Highlight all MindMap nodes containing 'login' in blue"
    Then the chat history should show that the LLM invoked the `highlight_mindmap_nodes` tool
      And when I switch to the MindMap tab (press `2`)
      And all tree nodes whose step text contains "login" (case-insensitive) should be highlighted in blue

  Scenario: AI tab is hidden when LLM is not configured
    Given the environment variable TESHI_LLM_API_KEY is not set or is empty
      And the TUI is running
    Then the top tab bar should not show an "AI [3]" option
      And pressing `3` should have no effect

  Scenario: Create and switch between multiple agents
    Given the AI tab is active with Agent 1 selected
    When I press `a` to create a new agent
    Then a new Agent 2 should appear in the left sidebar
      And the main panel should show Agent 2's empty conversation
      And Agent 2's status icon should show `○` (idle)
    When I type a message in Agent 2 and press `Enter`
    Then Agent 2's status icon should change to `●` (waiting)
    When I press `k` to switch to Agent 1
    Then the main panel should show Agent 1's conversation
      And Agent 2's status icon should remain `●` (waiting) in the sidebar
    When I press `j` to switch back to Agent 2
    Then the main panel should show Agent 2's conversation again

  Scenario: Close an agent
    Given the AI tab is active with at least two agents
    When I press `x` to close the selected agent
    Then that agent should be removed from the sidebar
      And focus should switch to the remaining agent
    When I press `x` with only one agent remaining
    Then a status message should say "Cannot close the last agent"

  Scenario: Agent shows AwaitingApproval status after queuing a file change
    Given there is an active conversation in the AI tab
    When I send a message that causes the LLM to invoke a mutation tool
    Then the agent's status should change to `AwaitingApproval`
      And the sidebar should show `◆` (cyan) for that agent
      And the Y/N confirmation prompt should appear in the footer
    When I press `Y` to accept
    Then the agent's status should return to `Waiting` or `Idle`
      And the sidebar icon should update accordingly


  Scenario: AI chat content wraps automatically without horizontal scrollbar
    Given there is an active conversation in the AI tab
    When the AI sends a reply containing a line longer than the panel width
    Then the text should wrap to the next line automatically
    And no horizontal scrollbar should appear in the chat area
