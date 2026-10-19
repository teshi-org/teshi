# language: en
@tui
Feature: Safe project switching
  Teshi keeps project state, recent history, and global requirements isolated while switching.

  Background:
    Given Teshi starts with isolated project A and recent project B

  Scenario: Keyboard switching loads project B
    Given the isolated projects are ready
    When the operator switches to project B with the project command
    Then the project header shows project B
    And the active Feature content comes from project B

  Scenario: Pointer selection opens a recent project
    Given the isolated projects are ready
    When the operator opens the project picker
    And the operator selects recent project B with the pointer
    Then the active Feature content comes from project B

  Scenario: Open path switches to another project
    Given the isolated projects are ready
    When the operator opens the project picker from the project header
    And the operator chooses Open path
    And the operator enters project B's directory
    Then the project header shows project B
    And the active Feature content comes from project B

  Scenario: Selecting the active project is a no-op
    Given the isolated projects are ready
    When the operator opens the project picker from the project header
    And the operator selects the active project
    Then the active Feature content remains from project A

  Scenario: Canceling the picker keeps project A active
    Given the isolated projects are ready
    When the operator opens the project picker from the project header
    And the operator cancels the picker
    Then project A remains active

  Scenario: A missing project path preserves project A
    Given the isolated projects are ready
    When the operator opens Open path
    And the operator enters a missing project directory
    Then the missing project error is visible
    And the active Feature content remains from project A

  Scenario: Switching from B back to A reloads A
    Given the isolated projects are ready
    When the operator opens Open path and enters project B's directory
    Then project B is active
    When the operator opens Open path from project B and enters project A's directory
    Then the project header shows project A
    And the active Feature content comes from project A

  Scenario: Invalid target configuration preserves project A
    Given the isolated projects are ready
    When the operator opens Open path
    And the operator enters a project with invalid settings
    Then the project preparation error is visible
    And the active Feature content remains from project A

  Scenario: A narrow picker keeps the Open path control usable
    Given the isolated projects are ready
    When the operator narrows the terminal
    And the operator opens the project picker from the project header
    Then recent project B is visible
    And the Open path control is visible
    When the operator chooses Open path
    Then the project path input is visible
    When the operator cancels the picker
    Then project A remains active

  Scenario: Failed preparation preserves edits after choosing Discard
    Given project A contains an unsaved Feature edit
    When the operator opens Open path
    And the operator enters a project with invalid settings
    Then the unsaved changes guard is visible
    When the operator chooses Discard and switch
    Then the project preparation error is visible
    And the unsaved Feature edit remains visible in project A

  Scenario: Stopping a running task switches only after its runner tree exits
    Given project A has a running external task
    When the operator opens Open path and enters project B's directory
    Then the running task guard is visible
    When the operator chooses Stop tasks and switch
    Then the project header shows project B
    And the active Feature content comes from project B
    When the operator runs project B's scenario
    Then project B's scenario passes
    And project A's runner tree has stopped without writing into project B

  Scenario: Canceling a switch while ACP stops keeps project A usable
    Given project A has a running ACP conversation
    When the operator opens Open path and enters project B's directory
    Then the running task guard is visible
    When the operator chooses Stop tasks and switch
    And the operator cancels the switch while ACP is stopping
    Then project A remains active
    When the operator opens Open path and enters project B's directory
    Then the project header shows project B
    When the operator starts an independent conversation in project B
    Then project B's independent Agent reply is visible
    And project A's late reply is absent
    And ACP process and conversation ownership are isolated

  Scenario: Switching projects closes a stalled native model stream
    Given project A has a stalled native conversation
    When the operator opens Open path and enters project B's directory
    Then the running task guard is visible
    When the operator chooses Stop tasks and switch
    Then the project header shows project B
    When the operator starts an independent conversation in project B
    Then project B's independent Agent reply is visible
    And project A's late reply is absent
    And the old native stream closed before project B's reply

  Scenario: Project session lists and resume stay isolated after switching
    Given project A and project B have separate saved conversations
    When the operator opens project A's session browser
    Then only project A's saved conversation is listed
    When the operator resumes project A's conversation
    Then project A's saved reply is visible
    When the operator switches to project B
    And the operator opens project B's session browser
    Then only project B's saved conversation is listed
    When the operator resumes project B's conversation
    Then only project B's saved reply is visible

  Scenario: Restarting in project B restores only B's history
    Given project A and project B have separate saved conversations
    When the operator switches to project B
    And the operator resumes project B's conversation
    And the operator quits and restarts Teshi in project B
    And the operator resumes the most recent project conversation
    Then only project B's saved reply is visible

  Scenario: Explicitly loaded legacy history remains unassigned
    Given unassigned legacy history exists alongside project conversations
    When the operator opens the default session browser
    Then legacy history is absent from the project list
    When the operator explicitly opens the legacy session browser
    Then only unassigned legacy history is listed
    When the operator loads the selected legacy conversation
    Then the legacy reply is visible with an unassigned notice
    When the operator quits to checkpoint the loaded conversation
    Then the saved legacy conversation still has no project owner

  Scenario: A shared global requirement reloads each project's own links
    Given both projects link different test points to the same global requirement
    When the operator opens the Requirements page in project A
    Then the shared requirement shows only project A's linked test point
    When the operator switches to project B
    And the operator opens the Requirements page in project B
    Then the same requirement shows only project B's linked test point
    When the operator switches back to project A
    And the operator opens the Requirements page in project A
    Then the shared requirement shows only project A's linked test point
    And global requirement and project association files remain unchanged
