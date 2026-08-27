# WinUI3 AutomationId conventions (application under test)

teshi WinApp mode prefers `uia:automation_id=...` selectors. Those IDs must be set **in the WinUI3 app**, not in the `.feature` file.

## Naming rules

| Rule | Example |
|------|---------|
| Stable across locales | `LoginButton` not `登录按钮` |
| PascalCase for elements | `SaveSettingsButton`, `UserNameTextBox` |
| Screen prefix when helpful | `Settings_NotificationsToggle` |
| Unique within the attached window | Avoid duplicate `Button1` defaults |

## XAML

```xml
<Button
    AutomationProperties.AutomationId="LoginButton"
    Content="Log in" />
```

## Code-behind (optional)

```csharp
MyButton.AutomationProperties.AutomationId = "LoginButton";
```

## Custom controls

- Expose **UIA control type** and **Name** where possible.
- Implement **Invoke**, **Value**, or **Selection** patterns for non-standard controls.
- Without UIA metadata, teshi and exported behave tests fall back to brittle `uia:path` or `uia:name` selectors.

## PR checklist (app repo)

- [ ] Every new interactive control has a non-empty `AutomationId`.
- [ ] IDs are documented or follow the table prefix convention.
- [ ] Renaming UI does not rename `AutomationId` unless intentional (update bindings/export).

## List and collection items

WinUI3 list items often expose only generic **Name** text without per-row `AutomationId`. That makes bindings fragile (locale changes, duplicate names, scroll position).

| Approach | When to use |
|----------|-------------|
| `AutomationId` per item | Preferred — e.g. `LibraryGameItem_{gameId}` in the app |
| `uia:control_type=ListItemControl;name=...` | Fixed fixture data only; document the risk in binding rationale |
| `assert_text` / exact Name | uiautomation **exact** Name match in exported behave tests |

Recommend a PR in the app repo to assign stable IDs to list rows before relying on them in regression tests.

## teshi workflow

1. Record bindings with [winapp-regression](../skills/winapp-regression/SKILL.md).
2. Commit `.teshi/step-bindings/*.json` with the feature file.
3. Export with `teshi export --target behave` when CI should run without teshi.
