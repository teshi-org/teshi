# language: en
@tui
Feature: Project switching failure boundaries
  Failed saves retain edits and stopped agents cannot leave tools in the old project.

  Background:
    Given isolated projects A and B are open for failure testing

  Scenario: Undo and redo cannot restore the previous project's content
    Given project A has an unsaved Feature edit
    When the operator requests project B
    And the operator chooses Save and switch
    Then project B becomes active and A's edit is saved
    When the operator undoes and redoes in project B's editor
    Then project B's editor still contains only project B's content
    And project B's source files remain unchanged

  Scenario: Recover a failed Feature save before switching
    Given project A has an unsaved Feature edit
    And the Feature save destination is unavailable
    When the operator requests project B
    And the operator chooses Save and switch
    Then the save error is visible and project A remains active
    When the save destination is restored
    And the operator chooses Save and switch
    Then project B becomes active and A's edit is saved
    And project B's source files remain unchanged

  Scenario: Reject pending tool permission before switching projects
    Given project A is waiting for ACP tool permission
    When the operator clicks the project header behind the permission prompt
    Then the permission prompt retains project A without approving the tool
    When the operator rejects the permission request
    And the operator requests project B
    And the operator chooses Stop tasks and switch
    Then project B has no stale permission request or old reply
    And project B's source files remain unchanged

  Scenario: Stop ACP tools after the agent exits normally
    Given project A has an ACP turn with an active tool process
    When the operator requests project B
    And the operator chooses Stop tasks and switch
    Then project B becomes active after the old tool stops
    And no old tool writes appear in project B

  Scenario: Switching projects does not approve a pending test-point review
    Given project A has proposed test points awaiting human review
    When the operator requests project B through the path picker
    And the operator cancels the busy project switch
    Then project A retains its unapproved test points
    When the operator requests project B through the path picker
    And the operator chooses Stop tasks and switch
    Then project B becomes active without accepting A's review
    And project B's source files remain unchanged

  Scenario: A stalled synchronous dispatch times out without changing projects
    Given project A has a stalled synchronous dispatch
    When the operator requests project B
    And the operator chooses Stop tasks and switch
    Then the stop timeout is visible and project A remains usable
    When the dispatch is allowed to finish
    And the operator requests project B
    Then project B becomes active after the dispatch exits
    And project B's source files remain unchanged

  Scenario: Open a project path when recent history is empty
    Given recent project history is empty
    When the operator opens the project picker from the header
    Then the empty history notice and Open path control are visible
    When the operator chooses Open path
    And the operator enters project B's directory
    Then project B becomes active through the path picker
    And project B's source files remain unchanged
