# M25 - Footer thinking-level indicator

Date: 2026-08-26. One feature, one commit. Closes the first open item on the
README's future row.

## What

The status bar now shows the active thinking level as a `think:<level>` part
between the cost and the model label, for example:

```
~/My Agent Space/Epsilon (main) · ↑12k ↓3.4k · R10k · CH83.3% · $0.043 · think:high · claude-sonnet-4-5
```

`Off` renders nothing, so a default session looks exactly like it did before.
The `/help` line changed from "current level shown in notices" to "current
level in the status bar", because now that is true.

## Why here

pi keeps the level visible in its status line; until now Epsilon only showed
it in the transient notice pushed when you cycle with Ctrl+T or run
`/think`. The user's rule for this project: any number or state shown on
screen must come from the same source the code acts on. So the design point of
this milestone is not the string rendering (trivial); it is where the value
comes from.

## Key decision: mirror field written at the choke point

The footer draws from `App`, but the level lives on the `Agent` behind an
`Arc<Agent>`, and `draw_status` receives `&mut App` without the agent handle.
Options considered:

1. Pass the `Agent` into `draw_status`. Touches the bench seam and every test
   harness call site for one string.
2. Store an `Arc<Agent>` clone on `App` and read through it at draw time.
   Works, but couples draw code to live agent internals and adds a second way
   for draw state and agent state to diverge later.
3. Mirror the level into one `App.thinking` field, written ONLY inside
   `set_thinking`, the same method that already funnels both write paths
   (`/think <level>` and Ctrl+T).

Option 3 won. One writer means display and execution cannot drift; the
key-path test `ctrl_t_cycles_the_thinking_ladder` now asserts
`app.thinking == agent.thinking_level()` after real key events. Bench/test
apps built through `over_ui` never touch an agent, so they keep constructing
with the default (Off), which renders nothing.

## Rendering detail

`FooterStats` gained `thinking_level: Option<ThinkingLevel>`.
`stats_parts` pushes `think:<level>` only when `Some(level)` with
`level.enabled()`, placed after cost and before the context part so the
context tones (warn/error) stay last next to the model id, matching pi's
ordering convention. `FooterStats::default()` yields no indicator, which also
keeps `fresh_session_renders_minimal_footer` honest.

## Test-caught bugs

None this time; both new tests passed first run after formatting. The M16-era
lesson held anyway: the key-path assertion went into
`ctrl_t_cycles_the_thinking_ladder` (which drives `handle_key` with synthetic
events) rather than a standalone unit test, because the failure mode it
guards against is a future refactor adding a second write path that skips the
mirror.

## Verification

```
cargo fmt --all -- --check          clean (applied once after edits)
cargo clippy --workspace --all-targets   0 warnings
cargo test --workspace              242 passed, 0 failed
cargo test --workspace --features real   256 passed, 0 failed
cargo run -q -p epsilon-cli -- --demo    completes
cargo check -q -p epsilon-cli --features real   compiles
```

New tests:

- `footer.rs`: `thinking_indicator_hides_when_off_and_shows_the_level_otherwise`
- `app.rs`: extended `ctrl_t_cycles_the_thinking_ladder` with the mirror-equality assert

## Attribution

Pattern from pi's status line (`thinking` badge). Implementation is original,
on our existing M16a footer parts seam. Docs updated in the same commit per
repo law: README status row + suite totals, CHANGELOG entry, ADR amendment
under "Superseded / deferred" (M25).
