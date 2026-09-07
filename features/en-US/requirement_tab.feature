# language: en

@cli
Feature: Requirements tab
  As a tester opening teshi without a project path
  I want the Requirements tab to load the user-level library
  So that I can read documents that were already saved

  Sample documents:
  - doc-12 auth/login.md titled Login in Sprint 12
  - doc-37 mobile/login.md titled Login, unassigned
  - doc-9 shop/checkout.md titled Checkout, unassigned

  Background:
    Given an isolated requirement library with sample login and checkout documents

  Scenario: Teshi without a project path still loads the requirement library
    When the tester opens teshi without a project path
    Then the Requirements tab lists documents "doc-12", "doc-37", and "doc-9"
    And the Requirements tab shows the body of document "doc-12"
