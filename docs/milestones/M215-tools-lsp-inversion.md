# M215 — the tool registry stops building a language server

**Ledger row A8, plan [`023`](../plans/023-tools-lsp-edge.md).** One crate stops depending on
another, and a policy moves to the layer that owns the flag that sets it.

## What

`epsilon-tools` carried a direct dependency on `epsilon-lsp` whose **entire** purpose was one match
arm: `Lsp::Auto` constructed a `DiagnosticsService` and handed it to the write and edit tools. That
is not a layering nicety — every consumer of `epsilon-tools`, including its tests and any future
frontend, linked a language-server client whether or not it wanted diagnostics, and the architecture
law says LSP communicates through contracts.

Nothing about the diagnostics *behaviour* changed. What changed is where the decision is made:

| Before | After |
|---|---|
| `epsilon_tools::Lsp` (`Off`/`Auto`) | `epsilon_cli::runtime::Lsp` — the frontend owns the flag, so it owns the policy |
| `register_builtins_with(registry, confinement, lsp)` matched the policy and built the service | `register_builtins_with(registry, confinement, diagnostics: Option<Arc<dyn DiagnosticsProvider>>)` takes the handle |
| the `ServiceConfig` with `scrub_secret_env` was assembled inside `epsilon-tools` | `epsilon_cli::runtime::diagnostics_for(policy)` assembles it, once, beside the construction |
| `crates/epsilon-tools/Cargo.toml:13` → `epsilon-lsp` | the edge is gone; `epsilon-cli` is the only dependent |

This is the shape `ToolContext::task_host` and `ToolContext::goal_host` already use: **the frontend
decides, the crate receives a handle.** No new crate, no new edge, no behaviour in contracts.

## Key decisions

- **The `Lsp` enum moved rather than died.** The plan first proposed keeping it in `epsilon-tools`
  because that is the smaller diff. It is the wrong home: with the construction gone, the crate no
  longer knows what a language server *is*, and an `Lsp::Off` exported by a crate that cannot spawn
  anything is a concept with no referent. Eight call sites, all in `main.rs`, all mechanical.
- **The service configuration went with it.** `scrub_secret_env` is the credential rule for a
  subprocess, and a crate that no longer starts a subprocess is the wrong place to state it. It now
  sits in `diagnostics_for`, which is the only place that builds a service for a run.
- **One helper, not two constructions.** `diagnostics_for(policy)` is the single home for "off means
  no provider, auto means a default service", so `build_runtime` and any later caller cannot disagree
  about what `--lsp` bought. `doctor` still calls `DiagnosticsService::default().discover()`: it
  *lists* servers rather than running diagnostics, takes no policy, and never shared this rule.
- **No new abstraction.** `DiagnosticsProvider` already existed in `epsilon-contracts`, and both
  tools already took `Option<Arc<dyn DiagnosticsProvider>>` through `with_diagnostics`. Plan 023's
  step 1 (write the seam) was already done before the plan was read properly — see the plan's own
  step 0.

## The plan's open question, answered by reading the code

Plan 023's `Limits` said the estimate assumed the diagnostics path was injectable in one step, and
named the thing that would grow the slice: *"if the write/edit tools cache a client per tool instance
rather than per call"*. They do not. The provider is injected once, at registration, into
`WriteTool::diagnostics` / `EditTool::diagnostics`; nothing reaches further than
`register_builtins_with`. The slice stayed one function's body plus a type move, as the plan's step 0
predicted.

## Verification

**Battery `m215-final`, GATE PASS** — seven gates, `audit/evidence/gate-m215-final.log`, run against
the committed bytes with the documentation already final. **1,634 tests passed across 65 binaries**
(summed from the `test result: ok.` lines), against M214's 1,633 across the same 65 binaries: **+1,
the new test, no other change** — which is what a behaviour-preserving inversion should look like.
Earliest verdicts from that run: check-links 1 s, check-lint-tables 0 s, fmt 4 s, clippy 4 s, test
179 s, demo 5 s, real-check 1 s (a warm target; the same seven gates on the same code before the
documentation was written are `audit/evidence/gate-m215.log`, kept because the pair is the evidence
that **only documentation changed between them** — identical totals, identical binaries).

Run with `CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_PROFILE_TEST_DEBUG=0` under the owner's standing
instruction to keep this drive free (M208's knob, measured at 4.19 GB for a full battery instead of
~33 GB). The trade is the usual one and it is disclosed rather than quietly applied: **a panic
backtrace from this round would be close to useless**, so a failure here would be re-run with debug
info back on before being believed.

**The dependency proof, measured rather than asserted from the manifest:**

```text
$ cargo tree -p epsilon-tools -i epsilon-lsp
error: package ID specification `epsilon-lsp` did not match any packages
help: a package with a similar name exists: `epsilon-llm`

$ cargo tree -p epsilon-cli -i epsilon-lsp
epsilon-lsp v0.1.0 (…\crates\epsilon-lsp)
└── epsilon-cli v0.1.0 (…\crates\epsilon-cli)
```

The first is stronger than the plan asked for. The plan wanted "nothing depends on it from there";
what it reports is that the package is **not in that crate's graph at all** — there is no path to
invert. The second shows exactly one dependent left, the frontend that decides.

**The new test, and its negative control.** `runtime::lsp_policy_tests::the_policy_decides_whether_a_provider_exists`
asserts that `Lsp::Off` yields `None` and `Lsp::Auto` yields a provider — the rule that moved, which
the type cannot express (`Option<Arc<dyn DiagnosticsProvider>>` says nothing about which policy
produced it). Because the API is new there is no pre-fix RED to run, so the control is a **reverted
fix**: with the `Lsp::Off` arm changed to build a service, the test fails with

```text
panicked at crates\epsilon-cli\src\runtime.rs:3325:9:
--lsp off must not build a service
```

— the sentence naming the property, not a type error. The arm was then restored and the file's mtime
touched, because a restored file cargo thinks is unchanged runs the *control* binary and reports the
opposite of the truth.

**Behaviour preservation for the feedback itself** is the existing pair in
`crates/epsilon-tools/tests/builtin_tools.rs` (M79): they inject a fake provider through
`with_diagnostics` and assert the block appears, and that no provider means no block. They exercise
the handle, which is exactly what the inversion leaves in place, and they pass unchanged.

## Limits

- **No test drives the CLI's own wiring.** `diagnostics_for` is asserted directly; that
  `build_runtime` passes it into `register_builtins_with` is read, not run — nothing in the tree
  builds a runtime under test (M212's `Limits` records the same gap).
- **The `scrub_env` closure is not asserted anywhere.** `epsilon-lsp` tests its own floor
  (`client.rs:342`), but that the frontend *installs* the stricter `epsilon_safety::scrub_secret_env`
  is visible only in the source. A regression here would silently weaken subprocess hygiene for
  language servers, and no test would fail.
- **`register_builtins_with_skills` still has no caller** (M97 says so on purpose: it exists for
  frontends that do not apply their own trust filter). Its signature changed with the others; its
  status did not.
- **Nothing user-visible changed, so `CHANGELOG.md` has no entry for this milestone.** The claim being
  made is precisely that a user cannot tell M214 and M215 apart except by reading `cargo tree`. A
  changelog line would be the kind of "notable change" that is not one.
- **The global `epsilon.exe` was not reinstalled**, and the reason is the same claim plus its
  evidence: the shipped behaviour is unchanged (identical suite totals, +1 test, and `--lsp auto|off`
  reaching the same diagnostics handle at the same call site), so a reinstall would rebuild a binary
  that differs from the installed one only in which crate constructs the service. The next
  CLI-visible change carries the reinstall.

## Found on the way, and not fixed here

The two files this slice edited (`epsilon-tools/src/lib.rs`, its `Cargo.toml`) were **CRLF in the
working tree while the repository stores LF**, so the first `git diff` reported a 185-line rewrite of
a 173-line file for a one-line change. Measured afterwards: **156 of 768 tracked files are CRLF on
disk**, and **three text files are CRLF in the committed blobs** (`agents/memory/README.md`,
`docs/milestones/M212-resume-opens-once.md`, `docs/plans/019-one-config-reader.md`). Git does not
report most of them because its stat cache says "unchanged" — the damage appears the moment someone
edits one. That is A9, and it is plan [`022`](../plans/022-line-endings.md); the two files touched
here were normalized to LF as part of this commit, and the rest is left to that slice rather than
smuggled in here.
