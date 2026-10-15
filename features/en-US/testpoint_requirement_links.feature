# language: en

@cli
Feature: Test point requirement links
  As a tester or agent maintaining a project's test point set
  I want to add, remove, and retarget requirement links on existing test points
  So that I can repair traceability without recreating test points in the TUI

  Sample documents:
  - doc-12 auth/login.md titled Login, in Sprint 12
  - doc-9 shop/checkout.md titled Checkout, unassigned

  Background:
    Given an isolated requirement library with sample login and checkout documents
    And an isolated project with a test point "tp-1" linked to document "doc-12"

  Scenario: Listing reports each test point with its links
    When the tester lists test points as JSON
    Then the command succeeds
    And the JSON test point "tp-1" has 1 link
    And the JSON link of test point "tp-1" points at document "doc-12"

  Scenario: Listing reports a test point that has no links
    Given an isolated project with a test point "tp-2" that has no links
    When the tester lists test points as JSON
    Then the command succeeds
    And the JSON test point "tp-2" has 0 links

  Scenario: Adding a link appends a resolved anchor
    When the tester adds a link on test point "tp-1" to document "doc-9" range "0:14" as JSON
    Then the command succeeds
    And the JSON test point "tp-1" has 2 links
    And the JSON link of test point "tp-1" to document "doc-9" is resolved

  Scenario: Removing a link keeps the other links
    Given test point "tp-1" also links to document "doc-9"
    When the tester removes the link of test point "tp-1" to document "doc-12" as JSON
    Then the command succeeds
    And the JSON test point "tp-1" has 1 link
    And the JSON link of test point "tp-1" points at document "doc-9"

  Scenario: Retargeting a link replaces the previous anchor
    When the tester retargets the link of test point "tp-1" to document "doc-9" range "0:14" as JSON
    Then the command succeeds
    And the JSON test point "tp-1" has 1 link
    And the JSON link of test point "tp-1" points at document "doc-9"

  Scenario: Mutating links returns an approved test point to proposed
    Given test point "tp-1" is approved
    When the tester adds a link on test point "tp-1" to document "doc-9" range "0:14" as JSON
    Then the command succeeds
    And the JSON test point "tp-1" is in review state "proposed"

  Scenario: Rewriting a requirement body makes an anchor stale and the point needs review
    Given test point "tp-1" is approved
    When the tester rewrites document "doc-12" so the linked quote no longer appears
    Then the command succeeds
    And the JSON test point "tp-1" has 1 link
    And the JSON link of test point "tp-1" is stale
    And the JSON test point "tp-1" is in review state "needs_review"

  Scenario: Adding a link to a document outside the store is rejected
    When the tester adds a link on test point "tp-1" to document "missing" range "0:14" as JSON
    Then the command fails
    And the JSON error code is "unknown_document"
    And the JSON test point "tp-1" has 1 link

  Scenario: Adding a link with an empty range is rejected
    When the tester adds a link on test point "tp-1" to document "doc-9" range "4:4" as JSON
    Then the command fails
    And the JSON error code is "invalid_range"
    And the JSON test point "tp-1" has 1 link

  Scenario: Adding a link past the end of the document is rejected
    When the tester adds a link on test point "tp-1" to document "doc-9" range "0:999" as JSON
    Then the command fails
    And the JSON error code is "invalid_range"
    And the JSON test point "tp-1" has 1 link

  Scenario: Adding a link to an unknown test point is rejected
    When the tester adds a link on test point "missing" to document "doc-9" range "0:14" as JSON
    Then the command fails
    And the JSON error code is "unknown_test_point"
    And the JSON test point "tp-1" has 1 link

  Scenario: Removing a link that matches nothing is rejected
    When the tester removes the link of test point "tp-1" to document "doc-9" as JSON
    Then the command fails
    And the JSON error code is "link_not_found"
    And the JSON test point "tp-1" has 1 link
