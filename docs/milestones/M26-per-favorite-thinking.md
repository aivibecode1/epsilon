# M26 - Per-favorite thinking levels

Date: 2026-08-26. Closes the second open item on the README's future row.

## What

Starring a model in the picker (Ctrl+F) now also remembers your current
thinking level for that model. Cycling favorites with F2 / Shift+F2 applies
the remembered level; cycling to a favorite without a memory keeps the
current session level. Unstarring drops the memory. The star notice now says
what it remembered: `★ <id> favorited at think:<level>`.

Storage is a new optional table in the existing global config file:

```toml
[picker.favorite_thinking]
"deepseek-chat" = "high"
"kimi-k2.6" = "off"
```

## Attribution and semantics

This is pi's scoped-models idea
(`packages/coding-agent/src/core/agent-session.ts:204`, `:1625-1647`):
scoped model entries carry an *optional* `thinkingLevel`. On cycle, an
explicit level overrides the session level; an absent one inherits it. Plain
`setModel` always inherits. Our favorites list plays the role of pi's scoped
models, so the mapping is direct:

| pi | Epsilon |
|---|---|
| `--models` scoped list | picker favorites (`[picker] favorite`) |
| optional per-entry `thinkingLevel` | `[picker.favorite_thinking]` row |
| `_cycleScopedModel` override-or-inherit | `App::cycle_favorite` |
| `setModel` inherits | `App::apply_model` (unchanged) |

## Key decision: memory mirror on App, not disk reads mid-keypress

The first implementation read the memory from disk inside
`cycle_favorite`. That broke under parallel tests (every TUI test shares the
real global config path; concurrent read-modify-write cycles clobber each
other) and put disk I/O on the hot keypress path. The fix follows lesson 16
from the harness-dev notes (inject, don't touch global state):

- `App` carries `favorite_thinking: Vec<(String, String)>`, loaded at startup
  alongside favorites/recents, refreshed by `toggle_favorite`.
- Cycling consults only the field.
- Config-layer functions keep their `_in(dir)` cores, fully unit-tested
  against temp dirs with no process state.

The failing-then-passing test story: the key-path test originally seeded the
memory through the real global config and failed intermittently because
another test's `save_model_id` write raced it. Seeding via the field made the
test deterministic AND made the production code faster. The test caught a
design bug, not just a test bug — same as M16's yank story.

## Test-caught bugs

1. **Global-config race (above).** Fixed by the mirror-field design.
2. **Clippy collapsible-if.** Nested `if let`s became one `find().and_then(parse)`
   chain, which is also simply shorter.

## Verification

```
cargo fmt --all -- --check          clean
cargo clippy --workspace --all-targets   0 warnings
cargo test --workspace              244 passed, 0 failed
cargo test --workspace --features real   258 passed, 0 failed
cargo run -q -p epsilon-cli -- --demo    completes
cargo check -q -p epsilon-cli --features real   compiles
```

New tests:

- `config.rs`: `favorite_thinking_set_clear_and_survive_other_writes`
  (round-trip, clear-on-unstar, mutual non-clobbering with favorite/recent keys,
  empty-id refusal). Table rows compared as a sorted set because toml map
  iteration order depends on crate feature flags.
- `app.rs`: `f2_applies_the_favorite_remembered_thinking_level` (override,
  inherit, footer-mirror equality — all through real F2 key events).

## Docs

README status row + suite totals (244/258), CHANGELOG entry, ADR amendment
under "Superseded / deferred" (M26), glossary already covers thinking levels;
favorites entry updated wording lives in the README row.
