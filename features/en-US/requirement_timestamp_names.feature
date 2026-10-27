# language: en
# Requirement: doc-24; authored only; not bound, executed or accepted.
@tui
Feature: Requirement timestamp names

  Scenario: Name a new document independently of identity
    Given an isolated requirement library
    When I create and save a requirement in Requirements
    Then its filename contains a UTC timestamp with microsecond precision and its title is shown in the tree

  Scenario: Preserve historical links
    Given a legacy requirement with test point links and a project association
    When I change its title, body and priority
    Then its path and stable ID remain unchanged and its links still resolve

  Scenario: Avoid equal timestamp collisions
    Given two new requirements with the same creation timestamp
    When I save both requirements
    Then neither body is overwritten and any creation conflict preserves the unsaved draft

  Scenario: Clock rollback cannot overwrite history
    Given an existing timestamp-named requirement in an isolated library
    When I create a requirement after the clock moves backward
    Then the existing requirement remains unchanged and the new filename reflects the current UTC wall time
