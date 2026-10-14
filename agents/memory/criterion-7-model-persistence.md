# Criterion 7 — model selection does not reliably survive a restart

Recorded 2026-10-14. **What is measured and what is a read are marked throughout.**

## ~~CONFIRMED ROOT CAUSE~~ — WITHDRAWN, it was `config_dir_override`, not `project_dir`

`epsilon-tui/src/app.rs:818`, in the function that saves a default:

    let dir = project_dir.or_else(epsilon_core::config::global_dir);

Three write sites share this one resolved dir:

- `:824` `save_model_id_in(&dir, id)` — the default model
- `:854` `save_favorite_models_in(&dir, &self.favorites)`
- `:860`/:869 `set_favorite_thinking_in(&dir, ...)` — per-model effort

**A project without its own `.epsilon/` therefore writes to the global dir**, so its
default, favourites and thinking levels apply to *every* project. Switching projects
does not restore each project's own configuration; the last project to save has
imposed its choices on all the others. That is criterion 7's exact requirement —
"switching between projects must restore each project's own configuration without
unintentionally affecting others" — and the leak is in the resolution rule, not in a
missing save call.

The function-level isolation is **verified** by
`a_projects_model_choice_survives_a_restart` (`config.rs`), which passes: the
round-trip works and one project's value does not leak into another's per directory.
So the leak is in the *caller's* dir choice, confirmed as a read only.

## Why it was wrong

`save_default_model` (`epsilon-tui/src/app.rs:811`) reads:

    let dir = self.config_dir_override.clone().or_else(epsilon_core::config::global_dir);

**`config_dir_override`, not `project_dir`.** It is a test seam — the doc comment says
"Writes to the override dir in tests, else the global dir — a global write the tests
must never make". There is no project directory in this path at all, so no write can
leak between projects. I read a field name wrong and built a confirmed-sounding
diagnosis on it.

**The real architecture is already what criterion 7 asks for:**
- global default: `~/.epsilon/config.toml [model] id`, written by "set default" and by
  `/model`, read at startup by `session.rs:93` and by `resolve_launch`'s fallback;
- per-project override: `<workspace>/.epsilon/config.toml [model] id`, read through
  `ProjectConfig.model_id` with precedence **above** the global fallback.

So global defaults *are* kept separate from project-level overrides, and the model
selection does survive a restart. **The failing selection is probably not the model at
all.** Unverified candidates, in order: reasoning effort, project settings, auth reuse.

## Fix direction (one place, not three call sites)

Decide the write directory once — "the project's own `.epsilon/` when the workspace
has one, the global dir otherwise" — and have all three writes use it. Do not resolve
it independently at each call site, which is the shape that produced this.

## What exists

- `epsilon-core/src/config.rs:165` `load_model_id_in(config_dir)` — reads
  `model.id` from `<config_dir>/config.toml`. Fail-open for reads.
- `:179` `save_model_id_in(config_dir, model_id)` — writes it, preserving unknown keys.
- `epsilon-runtime/src/lib.rs:5906` — the model is restored:
  `req.model_reference .or(project.model_id) .or_else(load_model_id)`.

So persistence is wired. The defect is **where the writes and the read disagree**.

## The defect

Two write sites, two destinations, one read, and two different fields:

| site | writes to | sets |
|---|---|---|
| `epsilon-tui/src/app.rs:824` ("set default") | `project_dir.or_else(global_dir)` | `self.default_model` |
| `epsilon-tui/src/app.rs:1355` (`/model` switch) | **global only** (`save_model_id`) | `self.model_id` |

And the only startup read is `epsilon-tui/src/app/session.rs:93`
`default_model: load_model_id()` — **global only**.

Three consequences:

1. A per-project default is written to `<workspace>/.epsilon/config.toml` and read
   back from the global config dir. Write and read disagree about *where*, so the
   value is not restored.
2. The field the runtime actually consumes (`model_id`) is only ever restored
   **transitively**, through `default_model`, which is a different field with a
   different meaning (picker ordering) that a user can set independently.
3. So the two can silently diverge: the picker opens on the right model and the run
   uses another, or neither matches what the user chose.

This is an architecture defect, not a missing workaround: there is no single source
of truth for "the model this project is set to".

## The test that settles it

Two locations, in `crates/epsilon-runtime/tests/` or the existing config tests:

1. Write a per-project default to `<workspace>/.epsilon/config.toml`; build a
   runtime for that workspace with **no** `--model`; assert the resolved candidate is
   the project's id. This is expected to **fail** today.
2. Control: a second workspace with no project config must **not** pick up the first
   project's value — the `.or_else(load_model_id)` chain makes the global dir a
   shared fallback, and this test is what stops the fix becoming cross-contamination.

Also worth asserting, because criterion 7 names it: switching from project A to
project B restores B's own model and leaves A's selection untouched.

## Fix direction (do not build before the test above fails)

One resolution order, in one place: explicit request → project config → global
default → catalog default. Write and read must use the same resolution, not one
function's guess at a directory. The `default_model` field should either disappear
or become a genuinely distinct "open second in the picker" preference that no path
conflates with the active model.

## What the executed test settled (2026-10-14)

`save_model_id_in` / `load_model_id_in` **round-trip correctly per directory**, and the
isolation control holds: one project's value does not leak into another's, and a global
value does not write itself into a project that has none. **The config functions are
not the defect.**

The project path is also wired: `ProjectConfig.model_id` (`epsilon-runtime/src/lib.rs:96`)
reads `[model] id` from `<workspace>/.epsilon/config.toml`, and `:5905` puts it ahead of
the global fallback with the right precedence — `req.model_reference .or(project.model_id)
.or_else(load_model_id)`.

## The corrected root cause — a code read, not yet measured

The defect is narrower and worse than "write and read disagree about where": **the TUI's
active model is never restored from anywhere.**

- `epsilon-tui/src/app/session.rs:93` restores only `default_model: load_model_id()` — a
  *different field*, used for picker ordering.
- Nothing initialises `self.model_id` from `save_model_id`'s value on startup. So the
  runtime would honour the saved model through `resolve_launch`, while the TUI's own
  header and `/model` state start from the catalog default. The two disagree, and the
  user re-picks because the UI shows a model the run is not using.

**The test that settles this**: start a TUI session with a saved `model.id` present and
assert `app.model_id` equals it, with a control that no saved value leaves the catalog
default in place. That test is expected to fail today.

## Status

- Config functions: **verified** by an executed test with a control (passes).
- Runtime project-over-global precedence: verified by reading `resolve_launch`'s chain.
- Model restoration end-to-end: traced through `main.rs:1181` → `resolve_launch` →
  `App::new(model_id)`; appears **correct**, so the model selection is probably *not*
  the failing selection.
- **The `or_else` cross-project leak: confirmed as a read, not reproduced by a test.**
- Nothing is fixed.

**Withdrawn, kept so they are not re-formed:**
1. the write/read "where" mismatch — functions verified correct by an executed test;
2. "the active model is never restored" — chain traced end to end and found wired;
3. the `or_else` cross-project leak — **I misread `config_dir_override` as
   `project_dir`**. There is no project directory in that path at all.

**This file has been wrong four times.** The model selection is now verified wired
end to end. Treat every remaining read here as a hypothesis, and get a reproduction
from the user before spending another window on criterion 7.


## FIXED (2026-10-14) — the OAuth refresh was unconditional

`refresh_oauth_before_use` (`epsilon-llm/src/providers/mod.rs`) exchanged the refresh
token on **every** stream attempt: it matched `Oauth { refresh, .. }` and threw
`expires` away, so it never asked whether the access token was still live. `auth.rs`
already stores `expires` with pi's 5-minute skew precisely so it could ask.

Fix: the decision is now `needs_oauth_refresh(dir, provider)`, which defers to
`auth::oauth_access_token_in` — the resolver that already applies pi's 60s margin — so
the expiry rule lives in one place. **`Expired` only, not `!Live`:** `Missing` must not
trigger a refresh, because there is no token to exchange. My first version used
`!Live` and the test's third assertion caught it.

Proven: `a_live_oauth_token_is_not_refreshed_but_an_expired_one_is` — live → no
refresh, expired → refresh, absent → no refresh. 182 `epsilon-llm` tests pass behind
`--features real`; fmt and clippy clean.

**Not proven:** the guard's wiring. The control (removing the guard from the caller)
left the test passing, because the test exercises the predicate, not its caller, and
the caller is `async` + network. The fix's effect on the caller is a code read. Making
it testable needs the exchange function injected as a seam.

**Still unexamined in criterion 7:** project settings beyond the model; credential
reuse on restart.
