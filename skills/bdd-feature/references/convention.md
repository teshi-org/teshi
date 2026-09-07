# Gherkin authoring conventions

Use these conventions for writing and reviewing features. Preserve explicit user requirements and the consuming project's runner/language conventions.

## Organize by behavior

- Name a file for the behavior or business capability, such as `task_creation.feature`.
- Give each Scenario a clear condition, action, and expected outcome.
- Group related scenarios under `Rule` when it makes a business rule clearer.
- Keep each scenario independently runnable: its Given steps establish its own preconditions, rather than depending on another scenario's execution.
- Use Background for genuinely shared setup. Keep setup short enough that a reader can understand the scenario without tracing hidden prerequisites.

A focused regression may contain one scenario. Split files when they cover unrelated capabilities or become hard to navigate; do not add or merge cases merely to reach a scenario-count quota. A short purpose statement is sufficient unless the project requires an As/I want/So that template.

## Keep an action coherent

A scenario should demonstrate one coherent behavior. Multiple When-group steps can be necessary to enter data and submit a form; multiple Then-group assertions may jointly establish the same result. Split independent decisions or outcomes when that improves clarity, not solely because of a keyword count.

```gherkin
Scenario: Valid task submission adds the task
  Given the user can create tasks
  When the user enters the task name "Fix login"
  And the user submits the task
  Then the task list contains "Fix login"
```

`And` and `But` inherit the intent of the preceding Given, When, or Then. For locator replay, separate actions that require separate bindings; do not combine switching a panel and asserting an item into one opaque step.

## Separate behavior from automation mechanics

Keep CSS, XPath, test IDs, UIA selectors, and arbitrary sleeps in bindings or step implementations. Describe the visible control or intended behavior in Gherkin. Preserve details that are themselves the tested contract: an API scenario may explicitly assert a status code or endpoint, and a navigation scenario may explicitly name its URL.

Use the project's existing step catalog before adding equivalent phrasing. `teshi steps catalog` discovers Teshi's catalog when available; also inspect the consuming runner's step definitions. Use concrete example values for ordinary scenarios. Use Scenario Outline with an Examples table when multiple data rows exercise the same behavior; angle-bracket placeholders need that context.

Match the project's natural language. This Teshi workflow uses English Gherkin keywords with English or Chinese step text and `# language: en`; do not invent translated keywords under that header. Avoid creating duplicate language variants unless the task or project conventions require them.

## Review the result

Check that the scenario has meaningful preconditions, an executable trigger, and an observable expected result. Reuse supported steps, identify missing implementations/bindings, and distinguish feature authoring from test execution. Writing a valid feature does not prove its steps run or its behavior passes.
