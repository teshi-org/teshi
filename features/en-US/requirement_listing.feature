# language: en

@cli
Feature: Requirement listing
  As a tester or agent using the requirement library
  I want to list and show documents by iteration or identity
  So that I can inspect the store without opening the TUI

  Sample documents:
  - doc-12 auth/login.md titled Login in Sprint 12
  - doc-37 mobile/login.md titled Login, unassigned
  - doc-9 shop/checkout.md titled Checkout, unassigned

  Background:
    Given an isolated requirement library with sample login and checkout documents

  Scenario: Named iteration JSON list includes only assigned documents
    When the tester lists requirements in iteration "Sprint 12" as JSON
    Then the command succeeds
    And the JSON list contains only document "doc-12"

  Scenario: Unassigned JSON list excludes assigned documents
    When the tester lists unassigned requirements as JSON
    Then the command succeeds
    And the JSON list contains only documents "doc-37" and "doc-9"

  Scenario: Empty named iteration still succeeds
    When the tester lists requirements in iteration "Missing" as JSON
    Then the command succeeds
    And the JSON list is empty

  Scenario: Iteration and unassigned filters cannot be combined
    When the tester lists requirements in iteration "Sprint 12" together with unassigned
    Then the command fails
    And no requirement documents are listed

  Scenario: Unique title shows the checkout document
    When the tester shows the document titled "Checkout"
    Then the command succeeds
    And the output is the checkout document body

  Scenario: Document id shows the login document
    When the tester shows document "doc-12"
    Then the command succeeds
    And the output starts with "# Login"

  Scenario: Relative path shows the checkout document
    When the tester shows the document at path "shop/checkout.md"
    Then the command succeeds
    And the output is the checkout document body

  Scenario: JSON show includes document metadata and body
    When the tester shows document "doc-12" as JSON
    Then the command succeeds
    And the JSON show envelope has id "doc-12", title "Login", path "auth/login.md", and iteration "Sprint 12"
    And the JSON show body starts with "# Login"
    And the JSON show envelope includes store_id and revision

  Scenario: Duplicate title is rejected as JSON
    When the tester shows the document titled "Login" as JSON
    Then the command fails with exit code 2
    And the JSON error code is "ambiguous_requirement_ref"
