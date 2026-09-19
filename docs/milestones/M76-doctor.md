# M76 — `epsilon doctor`: the on-ramp, made self-diagnosing

**Scope:** roadmap Phase 5 item 4 — *"one command that prints resolved model + credential source,
sandbox backend + enforcement, session file + lease holder, config paths, and the last failures. Half
of this audit's findings would have been self-diagnosing with it."*

## The problem it closes

The audit's headline finding was that the engine was complete and the *on-ramp* was missing: a user
could install the binary, run it, watch it reach no model, and have no way to find out why. M63 fixed
the worst of it (the real 401 and its reason now reach the terminal); this closes the rest, and it is
the audit reviewing itself — the criterion in `audit/01-usability-review.md` was that a diagnostic
command would have turned most of those findings into a sentence the user could act on.

## What landed

`epsilon doctor` (and `--doctor`) — a mode that runs **before the runtime is built** (like `auth`), so it cannot fail
on the thing it is diagnosing, takes no session lease, needs no network, and does not print the
startup banner over its own output. Exit code `0` normally, `1` when a blocker is found, so it is
usable from a script or a bug report.

```
epsilon doctor
==============

build
  version            0.1.0
  target             windows x86_64
  real providers     yes (network path compiled in)

model
  requested          deepseek-chat  (from saved config)
  resolved           deepseek/deepseek-chat-v3.1  (openrouter)
  wire               openai-compatible chat completions
  matched by         Partial
  credential env     OPENROUTER_API_KEY
  credentials        MISSING (none found (auth.json or environment))
  providers with keys deepseek

sandbox
  confinement        Auto
  platform backend   job object (process-tree kill + memory cap); no filesystem confinement on this platform

workspace
  root               C:\Users\Hp\Desktop\My Agent Space\Epsilon
  git repository     yes
  instructions       AGENTS.md (8430 bytes)
  project config     …\.epsilon\config.toml (absent)
  mcp servers        0

session
  dir                …\.epsilon\sessions
  newest             serve.jsonl (919 bytes, 14h ago)
  entries            2
  write lease        free

tools
  built-in           11 (bash, edit, find, glob, grep, ls, read, task, todo, webfetch, write)

blockers
  - no credential for 'openrouter' — run `epsilon auth login openrouter` (or set OPENROUTER_API_KEY in the environment)
```

That run is the real output from this repository, and it reproduces the exact trap the user hit:
the saved model id `deepseek-chat` resolves by substring to **OpenRouter's** row (the reference is
loose — `matched by Partial`), whose credential is missing, while a `deepseek` key sits in
`auth.json` unused. The report says so, names the provider, and gives the command. With
`--model deepseek` the same command shows the M63 substitution working:

```
  requested          deepseek  (from --model)
  resolved           deepseek-flash  (deepseek)
  replaced           ~deepseek/deepseek-v4-flash-latest (no credential for it)
  credentials        ok (stored in auth.json)
```

## Design rules (what makes the output worth trusting)

- **Facts from the same APIs the run uses.** The model is resolved through the launch's own
  precedence (`--model` → project config → saved) and `catalog::resolve_model_for_run`; the tool list
  is `epsilon_tools::builtin_registry` with the effective confinement; trust is the same
  `load_project_config` + remembered-decision pair the launch consults. The report cannot disagree
  with what the run will do.
- **No secrets, ever.** Credentials are reported as *presence* and *source* — `auth.json`,
  `environment (VAR)`, `not needed` — never a value. `doctor` output is exactly the kind of thing a
  user pastes into a bug report, and a test pins the absence of secret-shaped strings.
- **`unknown` over plausible.** Where the harness cannot know something it says so: an unresolvable
  model is `UNKNOWN — no catalog row matches this`, a lock file without a holder is
  `held (holder not recorded)`, and a stale lock is reported as *held* with the escape hatch named,
  because process liveness cannot be known from a file.
- **Sections that fail still print.** A missing session directory, an absent project config, or a
  failed read reports its absence; the report continues. A diagnostic that gives up halfway is worse
  than none.
- **Blockers say what to do.** Each one names the thing and the command: `epsilon auth login
  <provider>`, `epsilon --list-models` for an unknown id, `remove the .lock file` for a stale lease.

## What the tests caught

Two mechanisms had **no test that could fail** until the mutation pass was run — the credential
blocker and the lease lookup. The fixes are the interesting part:

- The credential test cannot assume which keys a machine has, so it picks a provider this machine
  holds **no** credential for (there is always at least one, since nobody holds keys for every
  provider in the catalogue) and asserts the invariant a user depends on: no credential ⇒ a blocker
  naming the remedy. A companion test asserts the other direction on whatever provider *is*
  credentialled — and returns early, honestly, when there is none.
- The lease lookup is pinned against the lock-file name M61's store actually writes
  (`<session>.jsonl.lock`), then removed, to prove nothing is cached between gathers.
- One earlier mutation was a **no-op**: both branches of the lease lookup computed the same path, so
  "disabling" one changed nothing. The mutation that does discriminate disables the function itself.

## Verification

| Mutation | Result |
|---|---|
| a missing credential stops being a blocker | `a_row_without_a_credential_always_comes_with_the_command_that_fixes_it` fails |
| the lease lookup is disabled | 3 lease tests fail |
| the unknown-model message loses its diagnosis | `an_unknown_model_argument_is_a_blocker_that_names_it` fails |
| `UNKNOWN` is replaced by a plausible-looking value | `absent_sections_are_reported_as_absent_never_invented` fails |

16 new tests: 12 unit (rendering, blockers, exit codes, the offline-build warning, holder-less
leases, absent sections, no-secret-shapes, failure ordering, age buckets, clipping) and 5 integration
in `crates/epsilon-cli/tests/doctor.rs` driving `gather` against real files — a seeded session with a
failed tool result, a held lease, an empty workspace, and the lock-name contract.

Gate: `scripts/gate.sh m76` — fmt clean, clippy zero warnings, **713 tests, 0 failed**, demo
completes, `real`-feature build compiles.

## Plan status

Roadmap Phase 5 item 4 is done. Remaining there: LSP diagnostics (item 1), workspace checkpoints
(item 2), sandbox `enforcement: full | partial` (item 3), the `app.rs` split (item 5), and the
optional trio (persistent PTY, prompt templates, session stats).
