---
name: bdd-feature-convention
description: BDD Feature file granularity and language conventions for Gherkin/Behave projects. Use this skill whenever working with .feature files — writing new ones, reviewing PRs that touch them, splitting or refactoring existing scenarios, or instructing an AI to generate/audit Gherkin scenarios. Also use when discussing BDD test structure, scenario atomicity, step language isolation, or Feature file organization with Playwright + Behave + WinUI3 automation. This skill replaces ad-hoc BDD guidelines with a consistent, reviewable convention.
---

# BDD Feature File Convention

## Three-Layer Structure

```
Feature file
└── Rule (optional, groups business rules)
    └── Scenario / Scenario Outline
        └── Given / When / Then Steps
```

| Layer | Granularity | Example |
|-------|-------------|---------|
| **Feature file** | Broad | `task_management.feature` |
| **Rule** (optional) | Medium | `Rule: Unauthenticated users cannot create tasks` |
| **Scenario** | Narrow | `Scenario: Task creation rejected when name is empty` |

Use `Rule:` when a Feature file has >5 Scenarios that can be grouped by business rule. Omit when ≤4 Scenarios.

## Feature File Specification

### Naming

```
{BusinessObject}_{CoreVerb}.feature

✅ task_creation.feature
✅ user_authentication.feature
❌ test_task.feature           ("test_" prefix is test thinking, not business thinking)
❌ ui_task_form.feature        (UI detail leaked into filename)
```

### Size Constraints

| Metric | Value |
|--------|-------|
| Scenarios per file | **5–15** (<5 merge, >15 split) |
| File length | **≤200 lines** (including blanks and comments) |
| Background steps | **≤4** (more → sink to fixtures) |
| Nested Rule count | **≤5** |

### Description

Every Feature must include a user story:

```gherkin
Feature: Task Creation

  As a project member
  I want to be able to create new tasks
  So that I can track work items to be completed
```

Business rules below the user story are recommended but optional.

## Scenario Atomicity

### Three Principles

**One Outcome** — Each Scenario verifies exactly one business result.

```gherkin
# ❌ Too coarse: verifying both creation success and list refresh 
Scenario: Create task and verify list
  Given user is on the task list page
  When  user submits task name "fix login issue"
  Then  task is created
  And   task appears in the list        ← second outcome
  And   task count increments           ← third outcome

# ✅ Split
Scenario: Valid task submission shows success
  Given user is on the task list page
  When  user submits task name "fix login issue"
  Then  system confirms task creation

Scenario: Created task appears in the list
  Given system has an existing task list
  When  user creates task "fix login issue"
  Then  the first list item is "fix login issue"
```

**One Trigger** — Each Scenario has exactly one `When` (one business decision).

```gherkin
# ❌ Two independent business decisions in one Scenario
Scenario: Login then create task
  Given user is not logged in
  When  user logs in                   ← decision 1
  And   user creates task "fix issue"  ← decision 2
  Then  task appears in the list

# ✅ Split
Scenario: Valid credentials login succeeds
  Given user is on the login page
  When  user logs in with valid credentials
  Then  user enters the main interface

Scenario: Authenticated user creates task
  Given user is logged in
  When  user creates task "fix issue"
  Then  task appears in the list
```

**Self-Contained** — `Given` must fully describe preconditions, not depend on other Scenarios' execution.

```gherkin
# ❌ Implicit dependency
Scenario: Edit existing task
  Given there is a task from the previous step  ← depends on execution order
  ...

# ✅ Self-contained
Scenario: Edit existing task
  Given a task "fix login issue" exists
  When  user renames the task to "fix registration issue"
  Then  the task name is updated
```

### Step Count Constraints

| Metric | Recommended | Alert Threshold |
|--------|-------------|-----------------|
| Total steps (Given+When+Then) | **3–7** | >10 → mandatory review |
| When group | 1 business decision | Split if any `And` is an independent action |
| Then group (incl. And) | **1–3** | >3 → check if multiple outcomes |
| Given group (incl. And) | **1–3** | >3 → consider Background/fixtures |

### `And` / `But` Semantics

`And` and `But` inherit the role of the preceding keyword. Do not count them independently — group them under their parent (`Given`, `When`, or `Then`).

A When-group is valid when the `And` steps are **constituent parts of the same business decision** (e.g., filling a form). If any `And` could stand alone as its own Scenario's `When`, split.

```gherkin
# ✅ One business decision (filling + submitting the form)
When  user enters task name "fix issue"
And   user sets due date to tomorrow
And   user clicks submit

# ❌ Two independent decisions, must split
When  user creates task "fix issue"
And   user marks task as complete  ← independent action
```

### Scenario Naming

Pattern: **[Condition] + Action/Event + Expected Outcome**

```
✅ Task creation rejected when name is empty
✅ Task list refreshes after admin deletes a task
❌ test_create_task              (code style, no outcome)
❌ verify form submission         (test language)
```

## Language Layer Isolation

Three strictly separated layers:

```
Feature file (.feature)        ← Business language only
        ↓
Step Definitions (steps/)      ← Bridge: calls PO methods
        ↓
Page Objects (pages/)          ← UI operations (Playwright/UIA)
```

### Forbidden in Feature Files

All of the following belong in Step Definitions or Page Objects, **not** in `.feature` files:

| Category | ❌ In Feature | ✅ Correct |
|----------|-------------|------------|
| UI control names | `When user clicks the button id="submit-btn"` | `When user submits the form` |
| automationId / testId | `Then "task-list-item" element is visible` | `Then task appears in the list` |
| CSS selectors / XPath | `When user clicks .btn-primary` | `When user confirms the action` |
| Database field names | `Given status=1 in tasks table` | `Given an in-progress task exists` |
| API paths | `Then POST /api/tasks returns 201` | `Then task is created successfully` |
| Explicit waits | `When user waits 3 seconds` | (encapsulated in PO) |
| Screenshot/log assertions | `Then screenshot matches baseline` | (in Step Definitions only) |

### Step Language Levels

| Level | Allowed in Feature? | Example |
|-------|-------------------|---------|
| **Business** — intent, who does what | ✅ | `Given user is logged in` |
| **Domain** — complex preconditions | ✅ | `Given there are 3 pending tasks` |
| **Implementation** — URLs, selectors, IDs | ❌ | `Given page URL is "http://..."` |

### Step Catalog Reuse

Before adding a new Step expression, check the existing Step Catalog. Do not create semantically equivalent variants:

```python
# Already in catalog:
@when('user creates task "{task_name}"')

# ❌ Do NOT add (equivalent variants):
@when('user creates new task "{task_name}"')
@when('user adds task "{task_name}"')

# ✅ OK to add (substantially different semantics):
@when('user creates task "{task_name}" with priority "{priority}"')
```

Always parameterize variable data — use `<param>` placeholders, never hardcode values in step text.

## Quick Decision Card

When unsure about a Scenario, self-check in order:

```
Q1: How many Whens?        → >1 → split
Q2: How many Thens?        → >3 → review if multiple outcomes
Q3: Total steps?           → >10 mandatory, 7–10 suggested review
Q4: Control/ID/XPath/API?  → sink to PO, rewrite in business language
Q5: Given depends on       → make self-contained
    previous Scenario?
Q6: Name understandable    → describe business outcome
    by non-technical       (not test code style)
    stakeholder?
```

## Review Checklist

Use this checklist during code review of any PR touching `.feature` files:

### Feature Level
- [ ] File name follows `{BusinessObject}_{CoreVerb}.feature`
- [ ] Description includes user story (As a / I want / So that)
- [ ] Scenario count is 5–15 (if outside, reason explained in PR)
- [ ] File length ≤200 lines

### Scenario Atomicity
- [ ] Each Scenario has exactly 1 When
- [ ] Each Scenario has ≤3 Then assertions
- [ ] Each Scenario has ≤10 steps (recommended ≤7)
- [ ] Given is self-contained (no "from previous step" dependency)

### Step Language
- [ ] No automationId / testId / XPath / CSS selectors
- [ ] No API paths (`/api/xxx`)
- [ ] No database field names
- [ ] Step expressions checked against Step Catalog, no semantic duplicates

---

*This document is a living convention. Edge cases or rule disputes are settled by Test Architecture Team review.*
