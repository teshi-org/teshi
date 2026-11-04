# Authoring regression acceptance

On October 20, 2026, the remaining authoring regression was extended to cover
renaming a requirement, reloading it, and deleting it using its stable ID. The
test checks that the ID remains reserved and the document cannot be read. After
another reload, its test point retains the original document reference, marks
that reference stale, and requires human review.

Validation used isolated projects, requirement stores, and application data:

- Authoring state unit suite: 27 passed.
- Real Windows PTY creation, dirty guards, and tree scrolling: three passed.
- Real Windows PTY title persistence and global deletion: one passed each.
- Requirements pointer interactions at normal and narrow sizes: 15 passed.
- Test-point pointer, pane scrolling, fields, review, and modal ownership: five passed.

The 25 authoring PTY cases used the checkout-built CLI from
`.teshi/artifacts/windows-input-20261020/bin/teshi.exe`; reports and viewport
artifacts live under `.teshi/artifacts/authoring-20261020` and in
`.teshi/artifacts/windows-input-authoring-pty-20261020.log`. Eight additional Vim
PTY cases are recorded in [Windows terminal acceptance](windows-terminal-input-acceptance.md).

P6 priority semantics remain deferred. These checks do not grant human review
approval to generated test points or certify the entire historical Feature matrix.
