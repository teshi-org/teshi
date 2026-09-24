# language: en
Feature: Complex report workflow
  The scenarios deliberately share a long prefix and then diverge so the
  MindMap layout can be inspected with deep paths and sibling branches.

  Background:
    Given the workspace is ready
    And the user is authenticated

  Scenario: Export succeeds
    Given the report is open
    When the date filter is applied
    Then the preview shows filtered rows
    When export is requested
    Then the archive downloads
    And the checksum is shown
    But no warning is visible

  Scenario: Export is rejected
    Given the report is open
    When the date filter is applied
    Then the preview shows filtered rows
    When export is requested
    Then a permission error is shown
    And an audit event is recorded

  Rule: Refresh recovery
    Scenario: Retry a stale report
      Given the admin report is open
      When refresh is requested
      Then refreshed rows appear
      When retry is requested
      Then the retry succeeds
