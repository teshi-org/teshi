# Known defects

Tracked defects that are quarantined so CI can gate releases. Each entry names the
test that documents the intended contract and how to re-enable it.

## Bracketed paste loses characters

The VT / ConPTY input path delivers a bracketed paste as individual key events
instead of one paste event, and a run of those keys is dropped while a frame is in
flight. Pasting `second line` produces `secline` in the requirement editor.

- Contract test: `vim_bracketed_multiline_unicode_paste_is_one_history_operation`
  in `crates/teshi-tui-runner/tests/requirements_vim.rs`.
- Status: skipped unless `TESHI_RUN_KNOWN_DEFECT_PASTE=1` is set.
- Regression window: present since `f271c4a`; the parser itself keeps split reads
  intact (`bracketed_paste_survives_a_split_read`).
- Fix direction: keep the paste payload intact between the reader thread and the
  event loop instead of degrading it to key events.
