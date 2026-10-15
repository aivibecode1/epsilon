# 17 — Persistence audit (2026-10-15)

**Provenance, stated first.** Written by a read-only subagent of the 2026-10-15 campaign
that finished this report and then exited with its batch before a provider error took the
final delivery; the file below is its report verbatim from its workspace copy. The
integrator spot-verified its two headline claims first-hand: the reasoning-effort gap (no
config key exists for a thinking default; `/think` touches only the live agent) and the
resumed-model display split (the frontend's `model_id` is seeded before composition and
never updated after `build_runtime` applies the session's model). Everything below is
checkable at the cited lines the same way.

---

# Criterion 7 — persistence audit, settled by code reads

Audited: 2026-10-15 · tree at HEAD · read-only.
Scope: (1) model selection, (2) reasoning effort, (3) project settings,
(4) execution preferences, (5) sessions, (6) credential refresh placement,
(7) config-key migration.
Method: file:line reads only. No cargo. No notes trusted — every claim below
is anchored to a line read in this session.

---

## 0. The two config layers, as they actually exist

**Global layer** — `~/.config/epsilon/config.toml`, resolved by
`epsilon_core::config::global_dir()` → `epsilon_llm::auth::global_dir()`
(`crates/epsilon-core/src/config.rs:140-142`; the M223 "one implementation"
rule is enforced by the source-scanning test at `config.rs:1309-1350`).
Readers, all `*_in(config_dir)` with the dir injected and all fail-open
(`Option`/default, never an error — `config.rs:18-19`):

| key | reader | writer |
|---|---|---|
| `[model] id` | `config.rs:165` | `config.rs:179` |
| `[picker] favorite` (cap 16) | `config.rs:259` | `config.rs:284`, `:338` |
| `[picker] recent` (cap 8) | `config.rs:643` | `config.rs:667` |
| `[picker.favorite_thinking]` | `config.rs:504` | `config.rs:527` |
| `[display] hide_thinking` | `config.rs:590` | `config.rs:607` |
| `[skills] disabled` | `config.rs:385` | `config.rs:426` |
| `[editor] command` | `config.rs:20` | (hand-edit only) |
| `[tui] theme` | `main.rs:1367` | (hand-edit only) |
| `[provider_custom.<n>]` | `config.rs:47` | (hand-edit only) |
| `[oauth.<p>]` | `config.rs:86` | (hand-edit only) |
| `~/.config/epsilon/history` | `config.rs:740` | `config.rs:750` |
| `~/.config/epsilon/auth.json` | `auth.rs` | `auth.rs` (0600, tmp+rename, `auth.rs:775-789`) |

**Project layer** — `<workspace>/.epsilon/config.toml`, read by exactly one
function: `epsilon_runtime::load_project_config`
(`crates/epsilon-runtime/src/lib.rs:154-285`), trust-gated
(`:168-210`, `TrustDecision` at `:125-137`). Everything else in the tree that
opens that path only hashes it
(`epsilon_core::config::current_project_config_hash_in`, `config.rs:248`) or
reports it (`crates/epsilon-cli/src/doctor.rs:372`).

Every write in the TUI resolves its destination as
`self.config_dir_override.clone().or_else(epsilon_core::config::global_dir)`
(`app.rs:815-818`, `:845-848`) — i.e. **global, or a test override**. There is
no production path that writes `<workspace>/.epsilon/config.toml`.

---

## 1. Model selection — **PARTIAL**

**What works (verified, the repo's claim holds for the resolution chain):**

- `ProjectConfig.model_id` reads `[model] id` from the workspace's own file:
  `crates/epsilon-runtime/src/lib.rs:222-226`.
- `resolve_launch` builds the precedence flag → project → global → catalog:
  `lib.rs:5955-5959`
  ```
  let candidate_raw = req.model_reference.clone()
      .or_else(|| project.model_id.clone())
      .or_else(epsilon_core::config::load_model_id);
  ```
  A second workspace with no project config reads `None` and does not inherit
  (`config.rs:867-899`, `a_projects_model_choice_survives_a_restart`, with its
  cross-contamination control). No leak.
- `/model <id>` writes the global default (`app.rs:1355`,
  `epsilon_core::config::save_model_id`) **and** records a `ModelChange`
  entry in the session through the single choke point
  `Agent::set_model_id` (`crates/epsilon-core/src/agent.rs:100-111`, doc at
  `:84-93`), so a resume prefers it. Pinned by
  `epsilon-tui/src/app/tests.rs:8559` `a_model_switch_is_recorded_where_a_resume_will_find_it`,
  which asserts through `SessionStore::context_settings`, not through the
  notice.
- A launch-time resume adopts the session's model with the right order:
  `resumed_model_to_use` (`lib.rs:5437-5456`) returns `None` when
  `model_explicit`, and `build_runtime` applies it at `lib.rs:5161-5169`.

**What does not work:**

**(a) The TUI's own model never follows a resumed session's model.**
`build_runtime` calls `agent.set_model_id(session_model)` (`lib.rs:5168`),
but the id handed to the frontend is a separate immutable string:
`main.rs:1206-1222` builds `SharedRun { model_id: launch.model_id.clone(), … }`
and `main.rs:1684` passes `model_id: shared.model_id.clone()` into
`TuiConfig`; `epsilon-tui/src/app/event.rs:37,52` seeds `App::new`.
`shared.model_id` is never updated after `build_runtime` returns. So the
header, the picker's current row, `/session` (`commands.rs:1341`) and
`/settings` (`commands.rs:2401-2404`) all report the launch model while the
turn runs on the session's model. This is precisely the "two sources of truth"
shape `agent.rs:84-93` was written to remove — it was removed for the *session
record*, not for the display half.

**(b) There is no write path for a project's own `[model] id`.**
`save_default_model` (`app.rs:814-835`) and `apply_model` (`app.rs:1355`)
both write global only. The only way a project gets its own model is
hand-editing `<workspace>/.epsilon/config.toml`. So "switching projects
restores each project's own configuration" works only for values a user typed
by hand into the file.

**Smallest correct fix.**
(a) One line in `main.rs:1684`: after `composed`, pass
`agent.model_id()` instead of `shared.model_id.clone()` — or better, have
`build_runtime` return the post-resume id. (b) Give the write sites one
resolver, `project_dir_or_global()` = `<ws>/.epsilon` when the workspace has
one, else `global_dir()`, and route `apply_model`'s save through it, with a
`[model] id` write into the project file. That is the "decide the directory
once, not at three call sites" direction the memory note already names.

---

## 2. Reasoning effort — **DOES-NOT-PERSIST**

- **No config key exists.** `epsilon-core/src/config.rs` has no thinking key
  of any shape; the only thinking persistence is `[picker.favorite_thinking]`
  (`config.rs:504-581`), which is per-*favorite*, not a default.
- **The picker applies session-only state.** `/think <level>` and the bare
  `/think` level picker both terminate in `App::set_thinking`
  (`crates/epsilon-tui/src/app.rs:1202-1210`):
  ```rust
  fn set_thinking(&mut self, agent: &Arc<Agent>, level: ThinkingLevel) {
      agent.set_thinking_level(level);
      self.thinking = level;              // "the ONLY writer"
      self.ui.push_notice(format!("thinking level: {}", level.as_str()));
  }
  ```
  Three writes: the agent, a mirror field, a notice. **No disk.**
  (Key path: `keys.rs:1136-1144`; command path: `commands.rs:782-812`.)
- **Every launch starts at the default.** `App::new` sets
  `thinking: epsilon_contracts::ThinkingLevel::default()`
  (`crates/epsilon-tui/src/app/session.rs:180`) — `Off`
  (`crates/epsilon-core/src/agent.rs:65`). Nothing initialises it from
  anywhere. `/settings` then reports the session's own value
  (`commands.rs:2400-2404`), which is always `off` on a fresh launch.
- **The session reader exists but nothing feeds it or reads it.**
  `SessionStore::context_settings` (`crates/epsilon-session/src/store.rs:2138-2160`)
  walks the selected branch and returns `thinking_level` (defaulting to pi's
  `"off"`), and the type's doc says so explicitly
  (`store.rs:46-69`). But:
  - **No production writer of `EntryKind::ThinkingLevelChange`.** The only
    occurrences in the tree are tests
    (`crates/epsilon-session/tests/session_store.rs:1162,1179,3681,3713,3718`,
    `crates/epsilon-contracts/src/session_entry.rs:447`) and the legacy v1
    *healing* reader (`store.rs:2613-2621`). `Agent::set_thinking_level`
    (`agent.rs:201-209`) records nothing — the exact asymmetry
    `set_model_id`/`record_model_change` fixed for the model.
  - **No production reader of the field.** `settings.thinking_level` appears
    only in `session_store.rs` tests (`:3692,3707,3725`).
- The repo's own record already admits the config half: STATUS.md:112-113,
  *"Persisting a default is deferred (no default-thinking config key)"*.

**Smallest correct fix.** Two independent halves, both small:

1. *Default level*: add `load_thinking_level_in`/`save_thinking_level_in`
   following `hide_thinking`'s idiom exactly (`config.rs:590-640`), key
   `[thinking] level`, and read it at `session.rs:180`
   (`.unwrap_or_default()`). Write it in `set_thinking` — the existing single
   choke point, so both `/think` and Ctrl+T persist.
2. *Session restore*: append `EntryKind::ThinkingLevelChange` inside
   `Agent::set_thinking_level` (the one-writer pattern `agent.rs:100-111`
   uses), then apply `settings.thinking_level` beside
   `resumed_model_to_use` in `build_runtime` so a resume comes back on the
   level it was left at. Without the writer, the reader is already there and
   will start working.

---

## 3. Project settings — what `ProjectConfig` reads

Enumerated from `load_project_config` (`crates/epsilon-runtime/src/lib.rs:154-285`):

| `<ws>/.epsilon/config.toml` key | lines | consumed by |
|---|---|---|
| `[model] id` | `:222-226` | `resolve_launch` model chain `:5955-5959` |
| `[[rules]]` capability/pattern/action | `:228-244` | layered onto the ruleset gate as a `"project"` layer |
| `[[extensions]]` dir | `:246-254` | extension attachment |
| `[[mcp_servers]]` name/command/args | `:256-282` | MCP host |

**That is the complete list.** Everything else is global-only:

| setting | global key / state | project override? |
|---|---|---|
| favorites | `[picker] favorite` (`config.rs:259`) | none |
| favorite thinking memories | `[picker.favorite_thinking]` (`config.rs:504`) | none |
| recents | `[picker] recent` (`config.rs:643`) | none |
| hide_thinking | `[display] hide_thinking` (`config.rs:590`) | none |
| theme | `[tui] theme` + `EPSILON_THEME` + `--light` (`main.rs:1364-1370`) | none |
| permission preset | `--preset` flag / `/preset` mid-session, **no key at all** | none (project `[[rules]]` is the project mechanism) |
| editor command | `[editor] command` (`config.rs:20`) | none |
| skills disabled | `[skills] disabled` (`config.rs:385`) | none |
| custom providers | `[provider_custom.*]` (`config.rs:47`) | none |
| OAuth client ids | `[oauth.*]` (`config.rs:86`) | none |

Per-setting judgement on whether a project override is meaningful, and the
smallest wiring:

- **`[model] id`** — already overridden; *needs a write path* (see §1b).
- **`[display] hide_thinking`** — **not meaningful as a project override.**
  It is a preference about how much reasoning trace the *reader* wants, its
  toggle saves globally by design (`keys.rs:2302-2303`), and the doc at
  `state.rs:906-912` calls it a display preference. Wiring would be a
  `project_hide_thinking: Option<bool>` on `ProjectConfig` plus
  `.unwrap_or_else(load_hide_thinking)` at `session.rs:60` — ~4 lines, but it
  would let a repo override the user's own reading preference, which is the
  wrong direction of trust.
- **`[tui] theme`** — **mildly meaningful**, and cheap: a repo's README
  screenshots and a light-background workspace. Smallest wiring: read
  `[tui] theme` in `load_project_config` into an `Option<String>`, pass it
  beside `theme_mode` in `TuiConfig`, apply at `event.rs:56-58`. Note
  `/theme` deliberately does *not* write config at all
  (`commands.rs:1823-1827`: *"the palette is a view choice, not session state…
  `EPSILON_THEME` and `[tui] theme` remain the way to make a default
  permanent"*) — that is a documented design decision, not a gap.
- **favorites / favorite_thinking / recents** — **not meaningful per
  project.** A favorite list is a machine-level preference about the models
  *this user* calls, and it is already capped at 16 (`config.rs:256`). If a
  project override is ever wanted, `[picker.favorite_thinking]` is the one
  worth having (per-repo default effort per model) and it would come free
  with the `[thinking]`-key work in §2 once `ProjectConfig` grows a
  `picker` table.
- **`[skills] disabled`** — **meaningful**, and the strongest of the
  "should be per-project" set: suppressing a skill that misbehaves in *this*
  repository should not suppress it everywhere. Smallest wiring: `disabled:
  Vec<String>` on `ProjectConfig` from a `[skills] disabled` array, unioned
  *under* the global list (project suppresses, global still suppresses) at
  the composition site.
- **presets** — see §4.
- **`[provider_custom.*]` / `[oauth.*]`** — **deliberately not meaningful.**
  Letting a repository's config file decide which provider Epsilon talks to,
  or which client id an OAuth ceremony uses, is a credential-routing decision
  that must not be settable by checked-in code. They stay global.

---

## 4. Execution preferences — split verdict

**PERSISTS — `hide_thinking`.** Written on the chord
(`crates/epsilon-tui/src/app/keys.rs:2302-2303`,
`epsilon_core::config::save_hide_thinking(app.ui.hide_thinking)`), round-tripped
with a control that other keys survive (`config.rs:994-1023`), and read once at
`session.rs:60`. Global scope; there is no project override.

**DOES-NOT-PERSIST — follow-up mode.** `QueueMode` lives on the agent
(`crates/epsilon-core/src/queue.rs:96-120`), default `OneAtATime`
(`crates/epsilon-core/src/util.rs:135`). The only writers are the wire
transports: `set_follow_up_mode` in `rpc_mode.rs:1122-1131` and
`epsilon-server/src/lib.rs:882` (a test). There is **no config key, no
session entry, and no TUI surface at all** — `Command` has no variant for it
(`crates/epsilon-contracts/src/command.rs:269-406`). The TUI's own follow-up
behaviour is not a mode at all: it is the Alt+Enter key
(`app.rs:1015-1019,1188-1197`).

**DOES-NOT-PERSIST — steering mode.** Same shape
(`queue.rs:81-98`; `rpc_mode.rs:1107-1116`; default `util.rs:135`). Readable
only through `Agent::session_state` (`crates/epsilon-core/src/session.rs:327-339`,
`session_state.rs:71-74`), which is how a wire client can observe it but not
how it survives a restart.

**DOES-NOT-PERSIST — permission preset.** `/preset <name>` goes to
`RulesetGate::set_preset` (`crates/epsilon-safety/src/gate.rs:225-240`), which
mutates an in-memory `Mutex` and clears the session memory; nothing is
written to disk. The only persistent inputs are the `--preset` CLI flag and
the project's `[[rules]]`. `epsilon_runtime::Launch.preset`
(`lib.rs:6198`) is a straight passthrough of `req.preset`
(`main.rs:1194`). Arguably *deliberately* so — `danger-full-access` must be
re-confirmed each launch (`commands.rs:1425-1432` holds a yes/no gate), and a
persisted elevation that silently reapplied on restart would be the worse
behaviour. The gap worth closing is the *lowering* presets
(`read-only`, `workspace-write`), where no safety argument applies.

**Smallest correct fix.** Add one table, `[queue] steering` / `[queue]
follow_up` (or fold into `[display]`), read at agent construction and
written by the RPC op — then give the TUI a `/queue` arm so the state is
reachable at all. For presets, persist only non-raising names and re-gate
`danger-full-access` on every launch.

---

## 5. Sessions — **PERSISTS for the path and the model; PARTIAL for the live switch**

**The right per-workspace file is opened (verified).**
`session_path_for` (`lib.rs:5188-5197`) makes the resume target the file the
launch writes to (A19). `Resume::Latest` (`--continue`) resolves
`latest_session_for_cwd(&sessions_root(workspace), workspace)`
(`lib.rs:4983-4988`), which unions the per-cwd directory
`sessions/--<cwd>--/` with the flat legacy root
(`crates/epsilon-session/src/discover.rs:390-395`, naming at
`crates/epsilon-session/src/time.rs:120-129`). The TUI's browser and `/new`
use the same root (`epsilon-tui/src/app/session.rs:219-220`,
`commands.rs:1609-1622`). The launch writes
`.epsilon/sessions/--<cwd>--/<ISO>_<token>.jsonl`
(`lib.rs:1619-1627`) — so a workspace's sessions never mix with another's.

**The recorded model is restored (verified).** `resumed_model_to_use`
(`lib.rs:5437-5456`) reads `store.context_settings().model`
(`store.rs:2138-2160`) over the *selected branch* — a `ModelChange` **or** an
assistant message, last one wins — resolves it through the catalog, and
`build_runtime` applies it under the `--model` override
(`lib.rs:5137-5169`). The precedence flag > session > project/config is
correct and documented at `lib.rs:5142-5146`.

**The live `/resume` does not do it.** The TUI's in-session switch is
`App::switch_session_file` (`crates/epsilon-tui/src/app/session.rs:427-512`)
→ `Agent::live_resume` (`crates/epsilon-core/src/session.rs:80-163`), which
repairs, builds the model view, restores history and re-attaches the store —
and never calls `context_settings()` or `set_model_id`. Everything else on
that path is careful (cwd mismatch note `:438-443`, usage accounts
`:451-467`, compaction digests `:471`, compaction count `:479-499`, name
`:507`, title `:509`); the model is the one thing missing.

**Smallest correct fix.** In `switch_session_file`, after `live_resume`
returns, read `context_settings()` and apply the same rule
`resumed_model_to_use` names — `if !model_explicit_there_is_no_such_flag_here
&& model != agent.model_id() { agent.set_model_id(m) }`. Extract the body of
`resumed_model_to_use` into a shared `apply_resumed_context(agent, store)`
so the two paths cannot drift (the `AGENTS.md` §2.2 one-rule argument).

---

## 6. Credentials — **guard placement: PARTIAL**

The store itself is sound. `auth.json` persists under the global config dir
(`config.rs:156-158`), written 0600 with tmp+rename under `auth.lock`
(`auth.rs:775-789`), and `store_refreshed_oauth_in` (`auth.rs:671-710`) is a
locked re-check that keeps a concurrent process's rotation rather than
clobbering it. Credentials are reused across restart correctly.

**The guard, as placed:**

- `needs_oauth_refresh` (`crates/epsilon-llm/src/providers/mod.rs:1336-1341`)
  is `matches!(auth::oauth_access_token_in(dir, provider), OauthToken::Expired)`
  — i.e. `expires <= now + 60_000` (`auth.rs:749`). `Missing` is deliberately
  excluded (`:1329-1333`), and the test pins all three arms
  (`mod.rs:1739-1771`).
- Its **only** consumer is `refresh_oauth_before_use` (`:1344-1376`), which
  re-checks it at `:1353`.
- The **call site does not use it**. `DirectProvider::stream` gates on a
  *different* predicate (`mod.rs:1480-1488`):
  ```rust
  if let Some(dir) = crate::auth::global_dir()
      && crate::auth::oauth_needs_refresh_in(&dir, provider,
            crate::oauth::OAUTH_MIN_VALIDITY_MS)
  { refresh_oauth_before_use(&dir, provider).await; }
  ```
  `oauth_needs_refresh_in` (`auth.rs:655-664`) is
  `expires <= now + min_validity_ms + 60_000`, i.e. it opens the door
  **~6 minutes** before expiry (pi's `DEFAULT_OAUTH_MINIMUM_VALIDITY_MS`,
  `oauth.rs:19-21`). The inner check then closes it unless the token is
  already `Expired` — **~60 seconds** before expiry.

**Consequence: two predicates with two different thresholds, and the narrower
one wins.** Between "expires in 6 min" and "expires in 60 s" the outer gate
says refresh, the inner says no, and the token goes on the wire as-is. The
5-minute pre-emptive refresh pi exists to guarantee is dead code. It still
*works*, just only in the last minute — which is where a long request is most
likely to be interrupted mid-flight.

**What the guard misses entirely:**

1. **The whole fallback chain.** `FallbackProvider::stream` resolves each
   candidate's key through `crate::catalog::resolve_api_key`
   (`crates/epsilon-llm/src/fallback.rs:102`) and hands it to
   `try_stream_direct` (`:115`, `mod.rs:1381-1449`), which has no refresh
   hook. `refresh_oauth_before_use` has exactly one caller in the tree.
   An expired OAuth credential on a `--fallback` candidate never rotates.
2. **GitHub Copilot can never refresh.** Its endpoint carries
   `client_id_env: ""` (`oauth.rs:83-88`) because a Copilot mint needs no
   client id — but `refresh_oauth_before_use` reads the env var before
   branching on style (`mod.rs:1364-1366`), and `std::env::var("")` is an
   error, so the function returns early every time. The one provider whose
   refresh is a plain re-mint is unreachable.
3. **Credential stores are global-only** — by design here, but it means the
   guard itself only ever resolves `global_dir()` (`mod.rs:1480`); there is
   no per-project credential layer to miss.

**An expired token does trigger re-auth correctly** (on the direct path).
`resolve_api_key` returns `None` for anything but `OauthToken::Live`
(`catalog.rs:706-711`), so `DirectProvider::stream` emits
`no API key for '<provider>' (model '<id>') - run epsilon auth login <provider> or set <ENV>` (`mod.rs:1496-1508`) rather than sending a stale
token, and a restart prints the same actionable diagnostic
(`lib.rs:5986-5998`). **But**: an expired OAuth credential makes
`credentials_ready` false, so `ProviderChoice::Auto` can fall to the faux
provider with only that diagnostic line
(`lib.rs:6061-6077`, `6079-6090`) — a silent real→offline downgrade for
someone whose login simply aged out.

**Doc defect found while reading:** the doc comment at `mod.rs:1307-1320`
("Attempt one direct-HTTP stream for `model_id` with an explicit key…") is an
orphan from `try_stream_direct`; consecutive `///` blocks mean it now
attaches to `needs_oauth_refresh` and describes the wrong function.

**Smallest correct fix.**
(a) Delete the duplicated inner predicate and make the call site's
`oauth_needs_refresh_in` the *only* gate — the fix that landed made the
function testable but put the decision back in the caller with a different
number. (b) Hoist the refresh to where keys are resolved so the fallback
chain is covered: call it once in `FallbackProvider::stream` before
`resolve_api_key`, or fold it into the `OauthToken::Live` arm at
`catalog.rs:706-711` (making the resolver async is the cost; a pre-flight
`ensure_oauth_fresh(provider).await` beside the chain is cheaper). (c) Skip
the client-id read when `endpoint.client_id_env.is_empty()` — one guard
unblocks Copilot. (d) Re-point the orphaned doc comment.

---

## 7. Migration of renamed config keys — **DOES-NOT-PERSIST (none exists)**

There is no migration path, no config-schema version, and no key-alias
table anywhere in the tree. Every reader is fail-open by contract
(`config.rs:18-19`: *"Missing file, table, key, or a non-string value → `None`
(never an error: a config typo must not break a session)"*) and every writer
is a read-modify-write over the parsed `toml::Table`
(`config.rs:187-190`, `:291-294`, `:437-440`, `:611-614`, `:674-677`), which
is what preserves unknown keys — and is pinned by
`save_then_load_round_trips_and_preserves_unknown_keys`
(`config.rs:791-815`) and `writing_a_skill_setting_leaves_every_other_key_alone`
(`config.rs:1212-1245`).

Consequences, stated plainly: a key renamed between builds is **silently
ignored** on read (falls to the default) and **silently preserved** on write
(never rewritten). No notice, no upgrade path, no error. `grep -rn "migrat"`
over `crates/` matches only *session-store* file-format migration
(`store.migrated()` / `persist_migration`, `lib.rs:5023-5035`), a different
thing entirely.

**Verdict and value.** This is a real gap with low user value, because the
failure mode is a *silent default* rather than data loss, and the set of
persisted keys is small and stable. If it is ever closed, the shape that fits
this codebase is a `const CONFIG_KEY_MIGRATIONS: &[(&str, &str)]` applied on
first read of the global file (old key → new key), reported as one startup
notice — not a schema-version framework.

---

## Ranked gap list

| # | state | verdict | why it ranks here |
|---|---|---|---|
| 1 | reasoning effort | DOES-NOT-PERSIST | set every launch, no key, no session restore; the only persistence is per-favourite |
| 2 | OAuth refresh guard | PARTIAL | two thresholds, the 5-min window dead; Copilot unreachable; fallback chain uncovered |
| 3 | TUI model vs resumed session model | PARTIAL | the header and picker name one model while the run uses another |
| 4 | live `/resume` + `/import` model restore | PARTIAL | launch-time resume restores it; the in-TUI switch does not |
| 5 | steering mode | DOES-NOT-PERSIST | no key, no surface, reset every launch |
| 6 | follow-up mode | DOES-NOT-PERSIST | same; the TUI's Alt+Enter is a key, not a mode |
| 7 | project overrides for display/picker settings | PARTIAL | favourites, hide_thinking, theme, favourite-thinking, presets have none |
| 8 | expired OAuth → Auto offline downgrade | PARTIAL | re-auth message is correct, but the launch can silently go faux |
| 9 | permission preset across restart | DOES-NOT-PERSIST | deliberate for elevation; only the lowering presets matter |
| 10 | config key migration | DOES-NOT-PERSIST | silent default, low value |

Verified correct, for the record: the resolution chain for the model
(flag > project > global > session, `lib.rs:5955-5959` + `:5437-5456`);
`hide_thinking`; the per-workspace session directory and the file a resume
opens; the credential store's reuse and its no-clobber rotation; the
`$HOME`/`global_dir` single-implementation rule.
