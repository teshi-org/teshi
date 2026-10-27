# language: en
# Requirement: doc-33; partially bound; see test results for scenario status.
@tui
Feature: Explore horizontal navigation

  Scenario: Reach long Step text
    Given a Step wider than its details pane
    When I press Shift and Right while the pane is focused
    Then the text scrolls horizontally without changing pane focus

  Scenario: Clamp at the content boundary
    Given Step details scrolled to their rightmost edge
    When I continue scrolling right then resize the pane wider
    Then the scroll position stays within the available content and the scrollbar reflects it
