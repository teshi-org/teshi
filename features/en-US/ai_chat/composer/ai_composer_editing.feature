# language: en
# Requirement: doc-85; business scenarios; terminal bindings follow when TUI steps are implemented.
@tui
Feature: AI composer editing
  The chat composer supports selection, undo, word editing, paste replacement and multi-line history navigation, including Chinese input.

  Scenario: Select all then type replaces the draft and undo-redo restores it
    Given the AI composer contains a multi-line draft
    When I select all in the composer
    And I type one character
    Then the draft contains only that character
    When I undo the edit
    Then the original multi-line draft is restored
    When I redo the edit
    Then the draft contains only that character again

  Scenario: Select all then delete clears the draft
    Given the AI composer contains a draft
    When I select all and delete
    Then the composer is empty

  Scenario: Select all then paste replaces the draft
    Given the AI composer contains a draft
    When I select all and paste a text block
    Then the draft is replaced by the pasted text

  Scenario: Extending a selection then cutting writes it to the system clipboard
    Given the AI composer contains a draft
    When I move to the start and extend the selection to the right
    Then the selected text is visible in the composer
    When I cut the selection
    Then the selected text is removed from the draft
    And the system clipboard contains that text

  Scenario: Word deletion handles mixed Chinese and English text
    Given the AI composer contains mixed Chinese and English text
    When I delete by word at the end of the text
    Then the English word is removed as a whole and Chinese is removed one character at a time
    And the caret stays at the deletion point

  Scenario: The composer selection takes priority over the chat selection for copy
    Given both the AI composer and the chat area have selections
    When I copy
    Then the composer selection is copied
    And the composer selection remains unchanged

  Scenario: Wrapped drafts move the caret by display row
    Given a long line in the AI composer is wrapped into three display rows
    When the caret is on the second display row and I move up
    Then the caret only moves between display rows
    And sent-message history is not recalled

  Scenario: A full-width slash starts command completion
    Given the AI composer is empty and the input method is Chinese
    When I type a full-width slash
    Then the composer shows a half-width slash
    And slash-command completion appears

  Scenario: An image placeholder is deleted as a unit and undo restores it
    Given the AI composer contains a placeholder for an attached image
    When the caret is after the placeholder and I press backspace
    Then the whole placeholder is deleted
    When I undo the deletion
    Then the placeholder and image attachment are restored
