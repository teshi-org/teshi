@auth @smoke
Feature: User login
  As a registered user
  I want to sign in with my credentials
  So that I can access my account

  Background:
    Given I am on the login page

  Scenario: Successful login
    When I enter username "alice"
    And I enter password "correct-password"
    Then I should see "Welcome, alice"

  Scenario Outline: Failed login
    When I enter username "<username>"
    And I enter password "<password>"
    Then I should see "Invalid credentials"

    Examples:
      | username | password |
      | alice    | wrong    |
      | bob      | 123456   |
