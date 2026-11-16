# language: en
@tui
Feature: Project switching persistence and identity boundaries
  Loading, saving and project aliases preserve the active project's state.

  Background:
    Given isolated projects A and B are open for failure testing

  Scenario: Invalid target encoding preserves an unsaved project
    Given project A has an unsaved Feature edit
    And project B contains a Feature with invalid encoding
    When the operator requests project B through the path picker
    And the operator chooses Discard and switch
    Then the target read error is visible and A's edit remains available
    And project B's source files remain unchanged

  Scenario: Unreadable target sources can be repaired before retrying
    Given project A has an unsaved Feature edit
    And a nested Feature in project B cannot be read
    When the operator requests project B through the path picker
    And the operator chooses Discard and switch
    Then the target read error is visible and A's edit remains available
    When the target read restriction is removed
    And the operator requests project B through the path picker
    And the operator chooses Save and switch
    Then project B becomes active and A's edit is saved
    And project B's source files remain unchanged

  Scenario: Recent-history write failure does not undo successful activation
    Given both projects are ready without unsaved edits
    And recent project history cannot be saved
    When the operator requests project B through the path picker
    Then project B is active with a recent-history persistence warning
    And project B's source files remain unchanged

  Scenario: Equal project names are distinguished by their paths
    Given recent projects have the same display name and different paths
    When the operator selects the other project's path in the recent picker
    Then the other project's Feature is active despite the shared name
    And both projects' source files remain unchanged

  Scenario: Directory aliases preserve the active project's unsaved edit
    Given project A has an unsaved Feature edit
    And project A has a filesystem directory alias
    When the operator requests project A through its path aliases
    Then the same-project notice preserves A's unsaved edit without confirmation
    And both projects' source files remain unchanged

  Scenario: A failed requirement draft save can be cancelled and retried
    Given project A has an unsaved requirement draft
    And the requirement save destination is unavailable
    When the operator requests project B through the path picker
    And the operator chooses Save and switch
    Then the save error is visible and project A remains active
    When the operator cancels the failed switch
    Then project A still displays its unsaved requirement draft
    When the requirement save destination is restored
    And the operator requests project B through the path picker
    And the operator chooses Save and switch
    Then B is active and the requirement draft is saved in the shared store
    And project B's source files remain unchanged

  Scenario: A failed test-point save retains edits and review state for retry
    Given project A has an unsaved test-point field edit
    And the test-point save destination is unavailable
    When the operator requests project B through the path picker
    And the operator chooses Save and switch
    Then the save error is visible and project A remains active
    When the operator cancels the failed switch
    Then project A still displays its unsaved test-point edit
    When the test-point save destination is restored
    And the operator requests project B through the path picker
    And the operator chooses Save and switch
    Then B is active and A's changed test point still requires human review
    And project B's source files remain unchanged
