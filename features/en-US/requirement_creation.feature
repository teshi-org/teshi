# language: en
# Requirement: doc-23; executable via teshi-requirement-cli-runner.
# 2026-10-27: 5/5 scenarios passed; see doc/requirements-testing-2026-10-27.md.
@cli
Feature: Requirement creation

  Scenario: Create from a file with an explicit project
    Given an isolated requirement library and a Markdown document titled 登录
    When I create a requirement from the document with title 登录, priority P1 and an explicit project, requesting JSON
    Then creation succeeds and JSON contains its stable ID, path, revision and planned project association

  Scenario: Create from stdin without implicit associations
    Given an isolated requirement library
    When I create a requirement from Markdown on stdin requesting JSON without a project
    Then creation succeeds and the requirement has no project association

  Scenario: Reject a mismatched title
    Given an isolated requirement library with existing documents
    When I create a requirement whose supplied title differs from its Markdown heading requesting JSON
    Then creation fails with a JSON diagnostic and existing documents remain unchanged

  Scenario: Concurrent creation preserves every document
    Given an isolated requirement library containing a deleted reserved ID
    When two clients concurrently create independent requirements requesting JSON
    Then both succeed with different IDs and paths and neither reuses the reserved ID

  Scenario: Failed persistence leaves no partial document
    Given an isolated requirement library whose index cannot be replaced
    When I create a requirement requesting JSON
    Then creation fails and no new Markdown document remains
