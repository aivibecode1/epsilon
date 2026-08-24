# M16c — Picker/palette parity (opencode patterns)

Date: 2026-08-24 · Status: shipped. Two-stage model picker, command palette,
draft stash, and the shared fuzzy matcher — opencode's dialog interaction model.

## What was built

### 1. Fuzzy matcher (`fuzzy.rs`, no deps)
Hand-rolled subsequence matcher per the handoff's ≤40-line rule, scoring ported
from pi's `fuzzy.ts`: case-insensitive in-order matching; word-boundary hits
(after space/-/_/./slash/: or string start) −10; consecutive-run bonus −5/char;
gap penalty +2/skipped char. Lower = better. Shared by stage-2 model filtering,
the command palette, and future list UIs.

### 2. Two-stage model picker (opencode `dialog-provider` → `dialog-model`)
- Stage 1: providers from the bundled catalog with 🔑 auth column, ● active mark,
  and a model count; sorted auth-ready-first then alphabetical.
- Enter descends into stage 2 (that provider's models, fuzzy-filtered as you type,
  active model floated to top); Esc walks back a stage before closing.
- Ctrl+F live-fetch retained on stage 2 (fetches that provider's live catalog).
- The legacy single-list `build_rows` remains for `/model <id>` direct application
  and recents handling.

### 3. Command palette (opencode `command-palette.tsx`)
- Ctrl+P opens a fuzzy-filtered list of ALL commands — slash commands plus direct
  actions (expand-toggle) — each with its keybinding hint on the right.
- Enter dispatches through the same `parse_command` path as typed slashes (one
  execution route, one set of tests); Esc closes.

### 4. Draft stash (opencode prompt.stash)
Opening any dialog stashes the in-progress draft + cursor; closing restores both.
Type half a prompt, hit Ctrl+P → /models → Esc, and your draft is still there.
Verified end-to-end by a key-path test (stash → palette → run → picker → Esc →
draft restored).

## Key-path tests
- Ctrl+P opens palette, stashes draft, first entry runs (/models), picker closes,
  draft restored.
- Two-stage picker: Ctrl+M → stage 1 → Enter descends → Esc back → Esc closed.
All drive `handle_key` with synthetic KeyEvents per house discipline.

## Decisions
- Palette entries execute by re-parsing their slash form instead of a parallel
  action enum: one code path, zero drift between typed commands and palette.
- Stage-1 filter is display-only (providers are few); stage-2 gets real fuzzy
  filtering because catalogs can be long.
- No new dependencies (fuzzy is ours; dialogs reuse the existing overlay pattern).

## Verification

```
cargo fmt --all -- --check              # clean
cargo clippy --workspace --all-targets  # zero warnings
cargo test --workspace                  # see totals below
cargo run -q -p epsilon-cli -- --demo   # completes
cargo check -p epsilon-cli --features real
```
