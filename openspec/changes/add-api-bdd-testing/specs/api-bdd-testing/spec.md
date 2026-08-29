## ADDED Requirements

### Requirement: One Jinja2 envelope per HTTP interface

Teshi SHALL treat each `*.json.j2` template as exactly one HTTP request. The template SHALL render to a JSON object containing `method`, `url`, and MAY contain `headers`, `body`, `extract`, `assert`, and `timeout_ms`. Default template root SHALL be the project `api/` directory, overridable in `teshi.toml`.

#### Scenario: Two calls in one step are two files

- **WHEN** a Python step definition invokes `call("create_user.json.j2")` then `call("get_user.json.j2")`
- **THEN** the sidecar SHALL perform two separate HTTP round-trips, each from its own envelope file

### Requirement: Two-pass Jinja2 render

The sidecar SHALL render request fields (`method`, `url`, `headers`, `body`, `timeout_ms`) without a `response` object, send the HTTP request, then inject `response` (`status`, `json`, `headers`, `body`) and render `extract` and `assert`.

#### Scenario: Extract sees the HTTP response

- **WHEN** `extract` contains `user_id: "{{ response.json.id }}"` and the response JSON has `"id": "42"`
- **THEN** scenario `vars` SHALL contain `user_id` equal to `"42"` after that call

#### Scenario: Request render does not require response

- **WHEN** the request pass runs
- **THEN** missing `response` MUST NOT cause the request render to fail solely because `extract` expressions are present in the source file

### Requirement: Scenario vars without call kwargs

Gherkin capture names from the matched step definition SHALL be merged into scenario `vars` before any `call` in that step. `call` SHALL accept a template identifier and MUST NOT require callers to pass extracted or captured values as keyword arguments. `extract` results SHALL be merged into the same `vars` for later calls and later steps in the same scenario. `vars` SHALL be cleared at the start of each scenario, then seeded from `teshi.toml` `[api]` and process environment mappings.

#### Scenario: Hidden fields use template defaults

- **WHEN** the step text does not capture `role` and the envelope uses `"role": "{{ role | default('member') }}"`
- **THEN** the rendered request body SHALL use `member`

#### Scenario: Next step sees extracted vars

- **WHEN** a previous API step extracted `user_id` and a later step template uses `"{{ user_id }}"`
- **THEN** the sidecar SHALL substitute the extracted value without the step definition passing `user_id` into `call`

### Requirement: Assert in the envelope

When `assert` is present, the sidecar SHALL evaluate each assertion after `extract` using Jinja2 against `response` and `vars`. Any failed assertion SHALL fail that HTTP call and the enclosing Gherkin step.

#### Scenario: Status assert fails the step

- **WHEN** `assert` requires status 200 and the response status is 500
- **THEN** the step SHALL be reported failed and an `http_exchange` SHALL include the failed assertion

### Requirement: Step marker and engine tags

API Gherkin steps SHALL place `[API]` immediately after the keyword. Teshi SHALL strip `[API]` before behave step matching. Engine tags `@api` and `@ui` SHALL be resolved as: if the Scenario has any of these tags, use only Scenario tags; otherwise use Feature tags. Mixed mode SHALL be both `@api` and `@ui` on that Scenario. A scenario whose resolved tags are only `@api` but which contains a step without `[API]` that is a UI step, or only `@ui` but which contains an `[API]` step, SHALL fail without executing remaining steps.

#### Scenario: Mixed tags start both engines

- **WHEN** a Scenario is tagged `@api` `@ui`
- **THEN** Teshi SHALL start (or attach to) both the API sidecar and the UI sidecar used by that project before dispatching steps

#### Scenario: Scenario tag overrides Feature

- **WHEN** a Feature is tagged `@api` and a Scenario is tagged `@ui`
- **THEN** Teshi SHALL treat that Scenario as UI-only and MUST NOT treat it as mixed

### Requirement: Mixed-scenario Teshi dispatch

For mixed scenarios Teshi SHALL walk Gherkin steps in order: `[API]` steps SHALL execute via the API sidecar; other steps SHALL execute via existing browser or WinApp bindings. Pure `@api` headless runs MAY execute the whole scenario through behave using the same helper library.

#### Scenario: Interleaved step kinds

- **WHEN** a mixed scenario has `When I click save` then `And [API] I create a user named "Ada"`
- **THEN** Teshi SHALL run the click through the UI path and the following step through the API sidecar

### Requirement: HTTP exchange events

Each HTTP attempt SHALL emit an `http_exchange` event including template path, rendered method and URL, request headers and body, status, response headers and body, duration in milliseconds, extract results, and per-assertion outcomes. Default emission SHALL redact sensitive header and field names aligned with browser network capture, plus names configured in `teshi.toml`.

#### Scenario: Exchange is correlatable to a step

- **WHEN** a Gherkin step performs two `call`s
- **THEN** two `http_exchange` events SHALL be emitted and both SHALL reference that step

### Requirement: Jinja2 include sandbox

Templates MAY include or import only files under the configured template roots. Templates MUST NOT execute arbitrary Python or access the host OS through Jinja2.

#### Scenario: Include outside template root is rejected

- **WHEN** a template includes a path that resolves outside the template roots
- **THEN** the sidecar SHALL fail the call without sending the HTTP request

### Requirement: No API step-bindings

API steps MUST NOT require `.teshi/step-bindings` entries. Those files remain for browser and WinApp locators only.

#### Scenario: API project has no http strategy rows

- **WHEN** an `@api` scenario runs with only `features/steps` and `api/*.json.j2`
- **THEN** Teshi SHALL NOT require a step-bindings file for those steps
