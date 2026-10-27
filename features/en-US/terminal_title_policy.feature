# language: en
# Requirement: doc-28; authored only; not bound, executed or accepted.
# Proposed behavior; product decision pending.
@tui
Feature: Terminal title policy

  Scenario: Proposed project and aggregate state
    Given three agent conversations with one running
    When I view the proposed terminal title
    Then it identifies the project and one of three conversations running without exposing conversation text

  Scenario: Preserve shell title until a decision
    Given the terminal title policy is undecided
    When I start or switch an agent conversation
    Then Teshi leaves the terminal title unchanged
