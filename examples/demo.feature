@auth   @smoke
Feature: User login
  Background:
    Given the a

  Scenario: Successful login
    Given ok
    When I enter username "alice"
    And I enter password "correct-password"
    Then I should see "Welcome, alice"

  Scenario Outline: Failed login
    Given I am on the login page
    When I enter username "<username>"
    And I enter password "<password>"
    Then I should see "Invalid credentials"

    Examples:
      | username | password |
      | alice    | wrong    |
      | bob      | 123456   |
