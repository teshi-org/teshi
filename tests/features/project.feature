@project
Feature: Project File Management
  As a BDD practitioner
  I want to open single files or entire directories of feature files
  So that I can work with projects of any size

  # --- Single file mode ---

  Scenario: Open a single feature file
    Given I run teshi with a path to a .feature file
    Then the file is loaded into the editor buffer
    And the cursor starts at the first BDD node

  Scenario: Start with an empty buffer
    Given I run teshi without any file argument
    Then an empty buffer is created
    And the status bar indicates a new buffer

  Scenario: Single file uses the same three-stage layout
    Given I run teshi with a single .feature file
    Then the MindMap tree shows the project root and step paths for that file
    And all three-stage view transitions work normally

  # --- Multi-file mode ---

  Scenario: Open a directory of feature files
    Given I run teshi with a path to a directory
    Then all .feature files are recursively discovered
    And the MindMap tree shows step paths from all discovered files

  Scenario: Feature files discovered in lexicographic order
    Given a directory contains "checkout.feature", "auth.feature", and "search.feature"
    When I open the directory in teshi
    Then the project feature list is ordered as: auth, checkout, search

  Scenario: Nested directories are scanned
    Given a directory contains subdirectories with .feature files
    When I open the top-level directory in teshi
    Then feature files in all subdirectories are discovered

  # --- Gherkin parsing ---

  Scenario: Parse Feature with tags
    Given a feature file begins with "@auth @smoke"
    And the next line is "Feature: User login"
    Then the parsed BddFeature has tags "auth" and "smoke"
    And the feature name is "User login"

  Scenario: Tags are stored in AST but not shown in tree
    Given a feature file has tags "@auth @smoke"
    Then the parsed BddFeature.tags contains "auth" and "smoke"
    And no tag node appears in the MindMap tree

  Scenario: Parse feature description
    Given a Feature header is followed by free-text narrative lines
    Then the parsed BddFeature captures the description lines
    And the description ends before the next structural block

  Scenario: Parse Background with steps
    Given a feature file contains a Background block with steps
    Then the parsed BddFeature has a background with the correct steps
    And each step retains its source line number

  Scenario: Parse Scenario with steps
    Given a feature file contains a Scenario block
    Then the parsed BddScenario has the scenario name
    And all steps under it are captured in order
    And each step has keyword and body text separated

  Scenario: Parse Scenario Outline with Examples
    Given a feature file contains a Scenario Outline with an Examples table
    Then the parsed BddScenario has kind ScenarioOutline
    And the Examples table headers and rows are captured

  Scenario: Examples stored in AST but not shown in tree
    Given a Scenario Outline has an Examples table with 3 data rows
    Then the parsed ExamplesTable contains 3 rows
    And no Examples node or table row nodes appear in the MindMap tree

  Scenario: Step line numbers preserved
    Given a step "When I search for a product" is on line 15 of the source file
    Then the parsed BddStep has line_number 15

  # --- Step index ---

  Scenario: Build step index across all files
    Given a project with multiple feature files is loaded
    Then a StepIndex is constructed from all steps in all files
    And the index key is the normalized step body text
    And background steps are included in the StepIndex

  Scenario: Step normalization ignores keywords
    Given Feature A has "Given I log in"
    And Feature B has "When I log in"
    And Feature C has "And I log in"
    Then the StepIndex has one entry for "I log in" with 3 usages

  Scenario: Step normalization trims whitespace and lowercases
    Given a step has leading indentation "    Given I Log In"
    Then the normalized body is "i log in"
    And the normalized body for "Given  I   log in" is "i log in"

  Scenario: Reuse count reflects actual usage
    Given the step body "I log in" appears in 5 distinct scenarios
    Then the StepIndex entry for "I log in" has 5 usage locations
    And each location references the correct feature, scenario, and step index

  # --- Edit-to-tree sync ---

  Scenario: Returning from Stage 3 triggers re-parse
    Given I edited a step body in the Stage 3 editor
    When I press the left arrow key to return to Stage 2
    Then the current feature file is re-parsed from the buffer
    And the tree reflects the updated structure

  Scenario: Saving in Stage 3 triggers sync
    Given I edited a step body in the Stage 3 editor
    When I press s to save
    Then the file is written to disk
    And the tree structure is refreshed

  Scenario: Step index and tree sync update after edit
    Given a step body "I log in" had reuse count 3
    And I change one occurrence to "I sign in"
    When the tree refreshes
    Then the tree path for that scenario uses "I sign in"
    And the StepIndex entry for "I log in" has 2 usage locations
    And the StepIndex entry for "I sign in" has 1 usage location

  Scenario: Adding a new step updates the tree
    Given I add a new step line in the Stage 3 editor
    When the tree refreshes
    Then the new step appears in the tree under its scenario
    And the StepIndex includes the new step body
