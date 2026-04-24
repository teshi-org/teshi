@demo @ui @github
Feature: GitHub UI automation demo for full BDD syntax
  As a QA engineer
  I want a complete Gherkin demo against GitHub web UI
  So that I can validate BDD syntax coverage before implementing a runner

  Background:
    Given the browser is launched with a clean profile
    And I navigate to "https://github.com"
    But I do not rely on any stored login session

  @smoke
  Scenario: Open login page and validate baseline UI elements
    When I click the "Sign in" entry point
    Then I should be on the GitHub login page
    And I should see the "Username or email address" input
    And I should see the "Password" input
    And I should see the "Sign in" button

  @happy_path
  Scenario: Search a public repository and open its details page
    Given I am on the GitHub home page
    When I search for "rust-lang/rust"
    And I open the "rust-lang/rust" repository result
    Then the repository header should show "rust-lang/rust"
    And the page should display repository navigation tabs

  @navigation
  Scenario: Navigate from repository Code tab to Issues tab
    Given I am on the "rust-lang/rust" repository page
    When I click the "Issues" tab
    Then the Issues page should be visible
    And the URL should contain "/issues"

  @outline
  Scenario Outline: Search with different keywords from the global search bar
    Given I am on the GitHub home page
    When I search for "<keyword>"
    Then search results should include "<expected_result>"
    And the page title should contain "Search"

    Examples:
      | keyword           | expected_result      |
      | rust-lang/rust    | rust-lang/rust       |
      | microsoft/vscode  | microsoft/vscode     |
      | torvalds/linux    | torvalds/linux       |

  @negative
  Scenario: Show validation when submitting empty credentials
    Given I am on the GitHub login page
    When I click the "Sign in" button without entering credentials
    Then I should remain on the login page
    And a sign-in error message should be displayed
