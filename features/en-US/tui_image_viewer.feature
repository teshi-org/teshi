# language: en

@tui
Feature: Responsive screenshot inspection
  Inspect original screenshot details without blocking terminal input.

  Background:
    Given an isolated project with recorded large and small screenshots

  Scenario: Close a viewer while a large image is being prepared
    Given Teshi is running with image graphics enabled
    When I open the screenshot viewer
    Then the viewer controls are available within 1000 milliseconds
    When I immediately close the screenshot viewer
    Then MindMap is interactive within 1000 milliseconds
    When I quit Teshi
    Then the process exits successfully within 1000 milliseconds

  Scenario: Inspect original pixels with zoom and pan
    Given Teshi is running with image graphics enabled
    When I open the screenshot viewer
    Then the large screenshot is ready in fit view
    When I choose original-size viewing
    Then the magnification is 100 percent
    When I pan right
    Then the visible source region moves right without changing the capture
    When I increase magnification
    Then the magnification is 125 percent

  Scenario: Preserve magnification when resizing and reset to fit
    Given Teshi is running with image graphics enabled
    When I open the screenshot viewer
    And I choose original-size viewing
    Then the magnification is 100 percent
    When I resize the terminal
    Then the magnification remains 100 percent
    When I reset the screenshot view
    Then the large screenshot is ready in fit view

  Scenario: A newer capture supersedes pending image work
    Given Teshi is running with image graphics enabled
    When I open the screenshot viewer
    And I immediately select the next capture
    Then only the small capture remains selected after pending work settles

  Scenario: Reject an image beyond the decode budget
    Given the first capture exceeds the decoded-image limit
    And Teshi is running with image graphics enabled
    When I open the screenshot viewer
    Then the image limit error and external-viewer action are visible
    When I close the screenshot viewer
    Then MindMap is interactive within 1000 milliseconds

  Scenario: Explain unavailable terminal graphics
    Given Teshi is running with image graphics disabled
    When I open the screenshot viewer
    Then the unavailable graphics reason and external-viewer action are visible
