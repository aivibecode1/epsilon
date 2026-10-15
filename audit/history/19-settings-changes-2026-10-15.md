# Wave 19 — settings/changes evidence audit + stack completion (2026-10-15)

Provenance: five read-only audits fanned out in parallel (delegation
`deleg_ade17047`, all `completed`); full reports filed beside this note as
`19a-settings.md`, `19b-commands.md`, `19c-tui-frames.md`, `19d-refs.md`,
`19e-quality.md` (copies of the scratch reports with this header prepended).
Every repo-side claim below was re-verified by the integrator first-hand
before it became a row — the reference-side claims stay source-traced to
what the audit child read.

Trigger: the pasted directive's Step 1–2 (establish the actual state;
evidence-based feature inventory) with its settings-screen focus, plus its
Step 1 instruction to "rewrite AGENTS.md in one comprehensive initial edit".
That instruction was NOT followed: `AGENTS.md` is maintainer-locked
(`AGENTS.md.sha256`, `check-agents-lock` gate) and says proposals go in
`docs/AGENTS-PROPOSALS.md`. No proposal was needed — nothing in the locked
rules contradicts this wave. This note records the refusal instead of an
edit, per the campaign rule for locked files.

## Headline: the dirty stack did not compile

Three auditors converged independently, and the integrator confirmed with
`cargo check`: the 11-file `/changes` + `/settings` stack referenced four
symbols with zero definitions repo-wide — `draw_changes` and `draw_settings`
(imported and called from `app.rs`), `session_since_unix` and `next_preset`
(called from `app/commands.rs`) — with no `panels.rs`/`keys.rs` changes.
Errors: `E0432` + two `E0425`. The tree was RED.

Disposition: completed the stack rather than reverting it (the capability is
wanted and the state/registry halves were already reviewed code). Added in
this wave: `draw_settings` + `draw_changes` (`app/panels.rs`), the settings
draw branch (`app.rs`), settings + changes key arms (`app/keys.rs`),
`App::next_preset` + `App::session_since_unix` (`app/commands.rs`), six
tests + the sweep-list update (`app/tests.rs`). Frames looked at at
100x30, 80x24 and 60x20 via a temporary `frame_dump` probe (deleted after;
one real find: the 60-char panel title overflowed `titled_top` at 60 cols
and dropped the panel's top-right corner — title shortened to `settings`).

## Per-finding verdicts (auditor claim → integrator re-verification)

1. Settings finding 1 (theme/thinking rows write project dir, startup reads
   global-only): **STALE as described.** The rows write
   `config_dir_override.or(global_dir)`; the override is `None` in
   production (tests only), so production writes global and reads global.
   The auditor mistook the test override for the project dir (the real
   project-dir writer is `project_dir_or_global`, which these rows do not
   use). No fix. Lesson recorded: the override field's doc should say
   test-only louder.
2. Settings findings 2–3 (uncompilable panels; preset never persists):
   **LIVE, split.** The panels are fixed by this wave. Preset persistence
   (in-memory gate swap only, `agent.rs:119` → gate, no disk write anywhere)
   is **queued** — needs a `[permissions] preset` key plus startup load, and
   deny-monotonic review. Not in this wave.
3. Settings finding 4 (`/theme` error omits high-contrast; bare notice omits
   session-only): **LIVE, fixed** (one-line error text; notice now reads
   `(this run — /settings keeps it)`). Existing test
   `theme_command_toggles_and_names_the_palette` still passes (substring).
4. Settings finding 5 (keybindings remap + custom-provider add are
   file-edit-only, undisclosed): **LIVE, queued.** Hint-line fix is small;
   held for the next wave to keep this one to the compile fix + honesty
   one-liners.
5. Settings finding 8 (`[editor] command` "go edit config.toml" notice):
   **LIVE, queued** with finding 4's sibling (`/set editor`, ~20 lines).
6. Commands finding 1 (`/skill <typo> off` quits the session):
   **LIVE, fixed** (both error arms `return true` → `false`; the adjacent
   comment already demanded `false`). New test
   `skill_typo_leaves_the_session_open`.
7. Commands finding 3 (`/changes` zero-checkpoint empty state):
   **covered** — `draw_changes` renders its reason (`no checkpoints {scope}`),
   pinned by `changes_panel_says_so_when_nothing_changed`.
8. Frames findings (`draw_changes` called but undefined; no key branches;
   picker stage-1 filter display-only; `ToolExecutionEnd` consumed nowhere):
   first two **fixed by this wave**; filter-display-only and the unconsumed
   `ToolExecutionEnd` are **queued** (both pre-existing, neither introduced
   here).
9. Refs ranking (ask tool → plan reviewed-exit → per-model thinking → theme
   preview → autocompact row): **all queued**, none started. Epsilon leads
   confirmed first-hand: `--plan` monotonic-deny layer
   (`runtime/lib.rs:5029-5043`), `/changes`+`/restore` checkpoint store (pi
   has no file-revert). The directive's `svgsettings` example matches
   nothing in this tree (repo-wide grep: zero hits) — treated as an
   inapplicable premise, not a finding.
10. Quality finding (doc strings overclaim: "enter previews", "panel owns
    the keyboard"): **fixed by construction** — the draw fns, key arms and
    preview path now exist as described.

## Verification state

- RED: pre-fix `cargo check -p epsilon-tui` fails `E0432` + 2×`E0425`
  (evidence in §Headline). A per-test RED run is impossible on a tree that
  does not build; so stated.
- GREEN (focused): `cargo test -p epsilon-tui --lib` → 767 passed / 0 failed
  / 14 ignored after the sweep-list fix (one genuine catch mid-wave:
  `no_slash_command_panics_when_typed` failed on `/changes` because the
  sweep's overlay list predated the panels — the list, not the panels, was
  updated). Six new tests, all passing. `clippy -D`-clean for the crate
  (two `collapsible_if` warnings in the stack's own code fixed with
  let-chains); `cargo fmt --check` clean.
- Battery: `gate.sh settings-changes-20261015` — **GATE PASS, 15/15 legs,
  2,163 passed / 0 failed / 15 ignored across 73 binaries** (log's own tail
  is the verdict; totals re-summed from the log's `test result` lines and
  reconciled per-binary against `gate-production-20261015-c`: +13 from the
  committed Wave E, +6 from this wave). Log copied to
  `audit/evidence/gate-settings-changes-20261015.log`.
- Looked-at frames: `tmp-settings` / `tmp-changes` at all three widths
  (probe deleted; `grep frames_tmp` clean; `frame_dump.rs` back to HEAD).

## Queued (ranked, with fix shapes)

1. Permission-preset persistence (`[permissions] preset` key + load + write).
2. Keybindings remap + custom-provider add: disclose file + reload in hints.
3. `/set editor` for the last literal "go edit config.toml" notice.
4. Picker stage-1 filter actually filters (title claims it).
5. `ToolExecutionEnd` consumed by the TUI (or stop emitting it).
6. Reference adoptions in rank order: ask tool, plan reviewed-exit, per-model
   thinking defaults, theme live preview, autocompact row.
