# language: en
# Requirement: doc-30; partially bound; see test results for scenario status.
@tui
Feature: External feature sync

  Scenario: Discover additions in the configured scope
    Given an isolated project opened recursively
    When another editor adds Features in the root and a subdirectory
    Then both Features appear without reopening the project

  Scenario: Refresh a batch of changes
    Given an isolated project with clean Feature buffers
    When another editor changes several Features and deletes one
    Then the changes are reflected as a batch and the deleted entry disappears

  Scenario: Keep a dirty deleted buffer
    Given a Feature with unsaved local edits
    When the file is deleted externally and I choose to keep local content
    Then my edits remain available and are not silently discarded

  Scenario: Defer changes during a run
    Given a Feature run is active
    When I request a manual refresh after external changes
    Then the active run keeps its original targets and synchronization resumes after it finishes

  Scenario: Retain the project on scan failure
    Given a project containing an unreadable directory
    When I request a manual refresh
    Then the UI reports the scan failure and retains the loaded project
