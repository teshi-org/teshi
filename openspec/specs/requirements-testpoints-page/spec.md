# Requirements-to-Testpoints Page (Retired)

## Purpose

The desktop/web Requirements→Testpoints page (FreeMind mindmap, mock HTML, and word-segment linking) is retired. Requirements gathering and scenario generation live in the TUI Agent pipeline. Desktop and web applications show Workspace only and do not generate FreeMind or mock HTML testpoint artifacts.

## Requirements

### Requirement: GUI does not provide requirements-to-testpoints generation

The desktop and web applications SHALL NOT present a requirements-to-testpoints generation page, FreeMind mindmap editor for test points, mock HTML generation UI, or an API endpoint that generates FreeMind/mock HTML testpoint artifacts. On startup, the application SHALL show the Workspace view (project welcome or editor workspace) without a Requirements/Workspace mode toggle.

#### Scenario: Startup shows Workspace

- **WHEN** the desktop or web app is launched
- **THEN** the Requirements-to-testpoints page SHALL NOT be shown as the default view

#### Scenario: Generate API is unavailable

- **WHEN** a client calls the former requirements generate endpoint (or equivalent Tauri command)
- **THEN** the system SHALL NOT accept and complete FreeMind/mock HTML testpoint generation
