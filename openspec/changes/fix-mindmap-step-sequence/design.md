## Context

`teshi-core::mindmap::build_index` describes its output as a step-sequence prefix trie, but its current keyword-state machine assigns parents according to Given/When/Then grouping. In particular, a `When` after any `Then` rewinds to the previous When group's parent, and consecutive Then-effective steps become siblings. The TUI renders the resulting arena faithfully, so the incorrect topology originates entirely in core.

## Goals / Non-Goals

**Goals:**

- Make every root-to-leaf scenario path preserve the exact source order of its Background and Scenario steps.
- Preserve prefix sharing and source-location aggregation for identical ordered paths.
- Exercise a complex, readable fixture whose topology is strong enough to validate tree layout.
- Keep the fix in the UI-independent core module.

**Non-Goals:**

- Adding Feature or Scenario container nodes to the trie.
- Changing node labels, keyword rendering, source navigation, highlighting, or filtering.
- Changing whether identical step text under the same parent is shared across files or keyword types.

## Decisions

1. **Use the previous inserted step as the next step's parent.** Each scenario starts at the root, inserts Background steps in order, then inserts Scenario steps in order. This directly models an ordered prefix trie and removes keyword-dependent topology. The alternative of retaining the keyword state machine and special-casing only `When → Then → When` would leave consecutive `Then` and Then-effective `And/But` out of source order.

2. **Test topology through labels and arena child links.** Node IDs are traversal-generated implementation details. Test helpers will resolve child nodes by label and assert ordered paths/children against the arena, making failures describe the visible tree rather than unstable IDs.

3. **Use one deliberately complex reusable Gherkin fixture.** It combines a Background, two top-level scenarios with a long shared prefix and different suffixes, multiple `When → Then` cycles, consecutive result-effective conjunctions, and a Rule-nested scenario. The core test reads the same `.feature` file that can be opened manually in Teshi, so automated topology checks and visual layout acceptance use identical data.

4. **Keep text-only deduplication unchanged.** `child_by_label` currently treats equal text under an equal parent as the same node and records multiple locations. Including keyword type in node identity would be a separate product decision and is unnecessary to repair ordering.

## Risks / Trade-offs

- **Existing users may prefer result assertions as siblings** → The displayed topology changes, but strict source order is consistent, navigable, and matches the documented prefix-trie model.
- **Deep scenarios produce taller trees** → This is the intended representation for sequential layout; shared prefixes still reduce duplication.
- **A broad fixture can obscure individual failures** → Add focused path assertions alongside branch and location-count assertions so failures identify the broken invariant.

## Migration Plan

No stored data migration is needed because the MindMap index is derived whenever a project is loaded or reparsed. Reverting the core parent-selection change restores the old topology if rollback is required.

## Open Questions

None for this fix. Feature/Scenario container nodes and keyword-sensitive node identity remain separate potential enhancements.
