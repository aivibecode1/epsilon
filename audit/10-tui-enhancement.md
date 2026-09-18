# TUI enhancement: viewport and composer

Owner priority: focus on TUI before continuing the compaction workstream.
This is a bounded correction, not a redesign or complete pi parity claim.

## Reproduced defects

- Editing the start of a long multiline draft displayed the last lines instead.
- Wide-character drafts used scalar-count cursor columns and clipped long lines.
- Composer height counted newlines instead of wrapped rows.
- Cached transcript rows were sliced and then scrolled again, leaving long chats blank.
- Streaming output was appended after an already-full visible window; long live tails
  also require their own offset within the combined history/live viewport.
- Small terminals had no explicit fallback.

## Implemented scope

Only new changes to `crates/epsilon-tui/src/app.rs`: draft/caret layout using existing
Unicode dependencies; caret-visible scrolling; composer growth capped at one third
of height; 60x12 minimum display; transcript slicing once; separately wrapped live
thinking/text with preserved newlines and viewport slicing.

Six TestBackend regressions exercise the production draw functions. Width matrix:
60x20, 80x24, 120x30, 160x40. Long stream uses 100 lines, exceeding its viewport.
The final package run passed 157 tests, zero failures (`evidence/tui-final-focused.log`).

## Verification and limits

First full battery failed formatting and the aws-lc-sys native test build; strict
Clippy, demo and real-check passed. Formatting was corrected. The final full battery passed all five gates: **803 passed,
0 failed, 1 ignored**, aggregated across 56 summaries. Evidence:
`evidence/gate-tui-final.log`. The ignored live-provider test was not run. This
is working-tree acceptance, not acceptance of an isolated commit.

No native terminal screenshots, interactive clipboard/IME tests, Linux/macOS runs,
full TUI integration review, or release qualification yet. Existing broad inherited
TUI changes are preserved; whole-file staging would absorb them and is not approved
by this report. Scoped change evidence: `evidence/tui-enhancement-change.patch`.

## Isolated M86 acceptance blocked

The later isolated-candidate review (deleg_9ba6a9a5) returned **FAIL**.
Report: [candidate review](evidence/m86-candidate-review.json). It found that
undersized fallback hides permission details while acceptance keys remain active.
Do not install or commit this candidate before fixing and testing that mismatch.

Five further objections cover wrapped tabs, caret position before a newline at
an exact-width boundary, HEAD scalar editing versus grapheme rendering, paged-up
viewport movement during streaming, and long-word clipping in the wrapper.
These are static review/arithmetic findings, not executed Rust regressions.
Existing green working-tree tests did not cover them. Candidate compilation and
full gates have not run; edition-aware rustfmt alone passed. TUI remains paused
while the independent F09 scheduling closeout finishes.

## Independent UX review disposition

The reviewer inspected an intermediate source snapshot, not the final patch.
Its proposed tests were not executed by the reviewer.

1. **Composer:** the principal clipping, caret-column and height defects are
   addressed by this slice's executed tests. Extra assertions for combining marks,
   moving back to draft end and cursor absence below minimum size remain useful.
2. **Modal focus — next priority:** parent source check confirms palette/picker
   key routing precedes permission handling (`app.rs:3275,3384,3564`), while
   permission renders above those overlays (`4039–4046`). Autocomplete still
   renders last (`4057–4058`). A visible approval may not own keyboard focus.
   Fix must align key/paste routing, draw order, hints and cursor ownership while
   preserving the interrupted filter/draft. Use inert approval-channel tests;
   no real tool execution is necessary.

   Follow-up: denial was runtime-reproduced while palette/model picker was open
   (`evidence/tui-modal-red.log`), then passed after moving the approval handler
   before background pickers (`evidence/tui-modal-green.log`). This only establishes
   key routing for denial, not paste/guidance/cursor ownership. An invalid rendering
   test scaffold was removed; red2–red5 logs contain compile errors and are not
   regression evidence. Full follow-up battery passed fmt and Clippy but workspace
   tests failed building aws-lc-sys (cl.exe 0xc0000142); no full-pass claim.
   Demo and real-provider compilation also passed. Final evidence is
   `evidence/gate-tui-modal.log`. After all compiler processes exited, external
   build artifacts were cleaned and repository target removed; see
   `evidence/tui-modal-cleanup.log`. No installation from this failed battery.
3. **Narrow footer:** parent source check confirms scalar-width measurements and
   unused truncation budget (`app.rs:4727–4740`). Keep model/offline and run state
   visible before optional cwd/usage; test synthetic long ASCII/CJK paths at
   widths 60 and 80. Width-aware hints remain open. Source-confirmed, not tested.

Remaining UX: frozen scrolling still counts appended logical rather than visual
rows, narrow footer priority, modal cursor/focus, wrap edge cases and keyboard
workflow review. Do not call these fixed by the new viewport tests.
