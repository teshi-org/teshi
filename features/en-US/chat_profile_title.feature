# language: en
# Requirement: doc-25; source-pinned PTY case replay; see results.
# Product decision approved.
@tui
Feature: Chat profile title

  Scenario: default title
    Given a chat using the built-in Default profile
    When I open the chat
    Then the title omits the Default suffix

  Scenario: custom profile title
    Given multiple custom agent profiles
    When I switch the active profile
    Then the title identifies the active Agent profile

  Scenario: A custom profile named Default stays identifiable
    Given a custom agent profile named Default
    When I open the chat using that profile
    Then the title displays Agent: Default
