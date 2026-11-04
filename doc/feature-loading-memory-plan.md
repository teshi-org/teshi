# Bounded Feature loading

## Contract

Explore displays only `.feature` files and their ancestor directories. Scanning
is distinct from parsing. A missing AST means unloaded, never an empty Feature.
Search and MindMap must display scope and index completeness. Unsaved edits must
survive cache eviction, scan failures, project switching, and background results.

## Target architecture

1. Store path discovery in a project-scoped disk catalog. Stream bounded batches;
   page directory children and search results. Keep stable file identities rather
   than using a changing list position as identity. Do not retain every directory
   page. Preserve locale scope and recursive scan semantics.
2. Load AST and source for the selected file through an explicit unloaded/loading/
   ready/failed state machine. Tag asynchronous results with project generation,
   file identity and file revision. Discard obsolete results.
3. Apply a byte-budgeted LRU to clean AST/source entries. Start with a configurable
   64 MiB clean-content budget; count actual retained allocations conservatively.
   This is not a total process memory guarantee. Limit workers and result queue
   bytes, and bound a single parse. Oversized files need scenario offsets and
   range loading rather than unlimited admission. Never silently evict dirty edits.
4. Keep editor-owned dirty buffers separate. Saving invalidates affected cache
   and index entries. Runners load their declared scope explicitly and stream
   cases; they must never infer the run scope from currently cached ASTs.
5. Default MindMap to the current Feature and label that scope. Directory or
   project scope queries a disk-backed relationship index and expands nodes
   incrementally. Do not build the full trie, node paths and TreeItems at startup.
6. Update full-text and reuse indexes one changed file at a time in the background.
   Commit file revisions atomically and expose incomplete/error states. Git should
   take paths and relevant source overrides, not a cloned project AST.

## Migration checklist

- [x] Extract path discovery from full-project parsing.
- [x] Stop background discovery reparsing already loaded files; preserve existing
      conflict-aware disk reload semantics.
- [ ] Introduce a disk catalog and bounded discovery batches with cancellation.
- [ ] Separate catalog identity from loaded AST and editor-buffer positions.
- [ ] Implement byte-budgeted content loading, eviction and oversized-file policy.
- [ ] Migrate editor, authoring tools, runner and project switching consumers.
- [ ] Introduce explicit MindMap scope and incremental graph queries.
- [ ] Remove full-project AST clones from Git refresh.
- [ ] Replace per-frame catalog hashing and whole-list line construction.
- [ ] Measure cold start, continued browsing, indexing and cancellation memory.

## Validation

Maintain scope, deleted-file handling, dirty-buffer conflicts and run coverage.
Test invalid/unreadable known files to prove discovery does not read them again.
Use the 10,000 Feature / 100,000 scenario / 3,000,000 step fixture (depth 8-12).
Readiness must observe an explicit scan-complete state, not a hidden filename in
collapsed directories. Persist terminal output and memory samples continuously,
stop navigation after a readiness timeout, and clean up only the spawned process.
Compare working-set and private-byte peaks, startup time, input response P95,
cache retained bytes and queue high-water marks. Single-run output batch counts
are not FPS. Accept only measured bounded memory across repeated file browsing.

## Current implementation and limitations

Directory startup now discovers paths in a background worker without parsing
Feature bodies. Selecting a file asynchronously loads its source and AST; clean
content is replaced when another file is selected. Switching is blocked while
edits or pending work need preservation. MindMap is scoped to the current Feature
and has an expansion budget. This is an initial single-file loading implementation,
not the completed target architecture.

Interactive reads reject sources larger than 2 MiB and ask users to split them;
range loading is not implemented. The in-memory catalog has a conservative 32 MiB
path budget and publishes a complete snapshot rather than incremental pages.
There is no byte-budgeted LRU. Background content results now compare canonical
path, length, modification time and available creation time across reading and
before installation. Changes reject the result with an explicit retry message.
The worker also rereads and compares source bytes after parsing, catching
same-length writes with preserved timestamps during that interval without source
IO on the UI thread. Changes after worker verification remain subject to metadata
checks; this is not an atomic filesystem snapshot or a complete revision protocol. Project
switches isolate results through per-instance channels.

Targeted runner and validation tools filter catalog paths before source reads.
Project-wide operations still traverse the discovered scope and report read errors;
the runner still materializes all selected run cases. Other remaining work includes
paged discovery, revision checks, consumer migration and measured memory accounting.
No dense-load performance or total-process bounded-memory claim is made.

## Reproducible loader baseline (2026-10-29)

Run `cargo test -p teshi-tui --lib --locked dense_loading_baseline -- --ignored --nocapture`.
The opt-in test creates a temporary depth-8 directory fixture with 10,000 files,
100,000 scenarios and 3,000,000 steps, asserts counts, scans paths, parses and
verifies each file, then browses the first 100 files three times. Temporary files
are removed by the fixture owner. This measures the loader, not rendered TUI input.

One Windows debug run: fixture creation 9,563 ms; scan 783 ms; sequential parse
and byte verification 22,714 ms; cumulative repeat-browse times 225/449/674 ms.
An external 100 ms sampler attached after process startup observed working-set
peak 12,660,736 bytes and private-byte peak 5,230,592 bytes. These are sampled
values for part of the test process lifetime, not launch-to-exit peak guarantees.
The fixture has repetitive text and uniform file sizes; it does not cover index,
Git, UI rendering, cold OS caches, input P95, or total application memory.

Selection requests carry a monotonically advancing instance-local revision, so
A-to-B-to-A navigation rejects the first A result. Installed content retains the
worker's disk stamp rather than recapturing a potentially newer stamp; subsequent
external-change polling can therefore detect an intervening ordinary write.
