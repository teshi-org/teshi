# language: en
@tui
Feature: Project switching failure boundaries
  Failed saves retain edits and stopped agents cannot leave tools in the old project.

  Background:
    Given isolated projects A and B are open for failure testing

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

  Scenario: Stop ACP tools after the agent exits normally
    Given project A has an ACP turn with an active tool process
    When the operator requests project B
    And the operator chooses Stop tasks and switch
    Then project B becomes active after the old tool stops
    And no old tool writes appear in project B

  Scenario: A stalled synchronous dispatch times out without changing projects
    Given project A has a stalled synchronous dispatch
    When the operator requests project B
    And the operator chooses Stop tasks and switch
    Then the stop timeout is visible and project A remains usable
    When the dispatch is allowed to finish
    And the operator requests project B
    Then project B becomes active after the dispatch exits
    And project B's source files remain unchanged
