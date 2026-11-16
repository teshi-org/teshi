# language: en
# Requirement: doc-22; authored only; not bound, executed or accepted.
@winapp
Feature: Winapp capability diagnostics

  Scenario: Prefer effective semantic controls
    Given a complete window snapshot with interactive controls
    When I inspect its capabilities
    Then the response reports the control count and recommends semantic UIA actions

  Scenario: Diagnose a container-only tree
    Given a complete snapshot containing only containers
    When I inspect its capabilities
    Then the response recommends supported visual observation and explains input permission

  Scenario: Do not infer absence from truncation
    Given a truncated snapshot without returned interactive controls
    When I inspect its capabilities
    Then the response reports uncertainty instead of claiming no controls exist
