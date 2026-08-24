# M14 — Harness maturity: TUI by default, provider auto-detect

Date: 2026-08-24 · Status: shipped (commit `73eda8d`). This dev log was backfilled
after the fact — M14 landed during a session where only the commit message survived;
this page reconstructs the record from the commit diff so the milestone sequence stays
complete. Every claim below is verifiable in `git show 73eda8d`.

## What was built

### 1. Plain `epsilon` launches the interactive TUI
Before M14 the bare binary printed a scripted demo (`Mode::Demo` was the default) and
the TUI required an explicit `--tui` flag. Now:

- `epsilon` → interactive TUI directly; the demo moved to an explicit `--demo` flag
  (preserved deliberately — offline scripted demos remain the CI smoke path).
- `USAGE` rewritten from a dense flag dump into a short task-oriented summary
  (plain run, `-p`, `--serve`, `--rpc`, `--demo`, flags list).

### 2. Provider auto-detection: real harness out of the box
`--provider` became optional (`Option<Provider>`) with this resolution order:

1. Explicit `--provider faux|real` wins.
2. Otherwise AUTO: if the resolved candidate model's credentials are ready
   (`catalog::credentials_ready`), or any tool-catalog model's env key is present,
   the run goes REAL against that provider.
3. Only when nothing is configured does Epsilon fall back to the offline faux
   provider — printing a banner that says exactly how to go real.

Rationale (pi/opencode parity): a fresh user with one API key in the environment gets
a working agent with zero flags. Before, they got a scripted demo and had to discover
`--provider real` themselves.

### 3. Slash-command repairs (the regression that earned key-path tests)
The picker's key handler was nested under a dead branch — slash commands silently did
nothing on the live input path even though unit tests passed (they tested the handler,
not the dispatch around it). Fixes shipped together:

- `/clear`, `/skills`, `/quit` wired and working.
- Unknown `/word` produces an error message instead of silence.
- Regression tests now drive the REAL key path (synthetic `KeyEvent`s through
  `handle_key`), which became the house discipline for all later editor/picker work
  (applied throughout M16/M17).

### 4. Small polish
Ctrl+M opens the model picker; a startup banner shows mode/provider/model at launch;
`.github/workflows/ci.yml` and `AGENTS.md` picked up matching wording tweaks.

## Lessons (recorded because they shaped later milestones)

- **Test the live path, not the helper.** M14's broken slash handling passed tests
  because the tests exercised a function the running app no longer called. Everything
  from M16b onward adds key-path tests that drive `handle_key` end-to-end.
- **Defaults are the product.** Auto-detecting one env var converted "compile it
  yourself demo" into "run it and talk to a model" — the single biggest usability jump
  between M13 and M16 despite touching almost no core code.

## Verification (from the commit; gates unchanged)

```
cargo fmt --all -- --check              # clean
cargo clippy --workspace --all-targets  # zero warnings
cargo test --workspace                  # green (136 at M14 close; M15 log confirms "was 136")
cargo run -q -p epsilon-cli -- --demo   # demo preserved behind explicit flag
```

## Related

- `docs/milestones/M15-tui-overhaul.md` — the usability overhaul this unlocked.
- ADR-018 (model selection precedence) — M14 added the provider half of auto-detect.
