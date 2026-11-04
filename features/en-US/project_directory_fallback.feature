# language: en
@tui
Feature: Project directory fallback
  Missing features defaults to root files; explicit recursion includes nested files.

  Background:
    Given an isolated launch directory and project xxx
    And the project uses isolated configuration and session data

  Scenario Outline: Load root files when features is absent
    Given project xxx has no features directory
    And project xxx has “根层功能.feature” in its root
    And project xxx has “其他功能.feature” in another directory
    When I launch “<command>” in “<directory>”
    Then the Feature list contains exactly “根层功能”
    And project xxx has no features directory

    Examples:
      | directory | command   |
      | 项目 xxx  | teshi     |
      | 项目 xxx  | teshi .   |
      | 启动目录  | teshi xxx |

  Scenario Outline: Recursively load root and nested files when features is absent
    Given project xxx has no features directory
    And project xxx has “根层功能.feature” in its root
    And project xxx has “其他功能.feature” in another directory
    When I launch “<command>” in “<directory>”
    Then the Feature list contains exactly “根层功能” and “其他功能”
    And project xxx has no features directory

    Examples:
      | directory | command                 |
      | 项目 xxx  | teshi -R                |
      | 项目 xxx  | teshi . --recursive     |
      | 启动目录  | teshi xxx -R            |
      | 启动目录  | teshi xxx --recursive   |

  Scenario Outline: Open an empty project when no Feature files exist
    Given project xxx has no features directory
    When I launch “<command>” in “项目 xxx”
    Then the Feature list is empty
    And project xxx has no features directory

    Examples:
      | command           |
      | teshi             |
      | teshi -R          |
      | teshi --recursive |
