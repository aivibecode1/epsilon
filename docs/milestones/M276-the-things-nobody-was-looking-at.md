# M276 — what nobody was looking at: a corrupted byte class, two ungated code paths, and the screen that said nothing

**Scope:** a security sweep of the four reference harnesses plus five independent read-only
reviews of this codebase, and the fixes they earned. Not one feature; **nine defects**, of
which three were reachable by opening a repository you did not write.

The title is the lesson. Every item below was invisible to the seven gates this project runs,
and each one was invisible for the same reason: **the thing that would have caught it was not
looking at the right thing.**

---

## What

| Area | Defect | Severity |
|---|---|---|
| Trust | `[[mcp_servers]]` in a project config spawned arbitrary binaries at startup, ungated | **critical** |
| Trust | Repository prompt templates loaded ungated and re-dispatched as harness commands (A32) | high |
| Trust | `.agents/prompts` was invisible to the trust-requiring-resource detector | high |
| Durability | `SessionStore::append` called `File::flush()`, which is a no-op — no append was ever fsync'd | high |
| Accounting | Bedrock and Mistral parsed cached-token counts and never set `cache_reported`, so the header printed `CH n/a` beside live cache traffic | high |
| Accounting | `session_usage()` lost a failed run's tokens while `session_usage_by_model()` kept them | medium |
| Crash | `/tree` sliced a `String` on a **byte** index computed from a **column** budget — a panic on any non-ASCII row | high |
| Invariant | The permission dialog silently swallowed every key it did not bind, and guidance mode silently swallowed the editor vocabulary | high |
| Invariant | `guidance_mode` was modelled, entered, and **drawn nowhere**; the caret vanished while you typed into it (A42) | medium |
| Display | Five glyphs were string literals instead of repertoire entries, so the fold markers — the whole point of the tree — rendered as boxes on the default Windows setup (A43) | medium |
| Data | 15 C0 control bytes in tracked files: a letter replaced by the C0 character of its own C escape name (`e`→0x1B, `a`→0x07, `b`→0x08, `f`→0x0C) | medium |
| Tooling | `gate-until-pass.ps1` named the wrong gate in its failure fingerprint, twice (A46) | medium |

---

## The three that mattered most

### 1. A repository could execute a binary on open

`build_runtime_and_gate_runner` gated skills through `skill_roots_trusted`, extensions through
`extension_roots_trusted`, and the sibling `extension_dirs` block **with a comment saying an
untrusted config must not smuggle code in through its own declarations**. Then:

```rust
let mut all_mcp: Vec<McpServerConfig> = mcp_servers.to_vec();
all_mcp.extend(project.mcp_servers.iter().cloned());   // ← not gated
```

`McpServerConfig::command` is a bare `String` parsed straight out of the repo's TOML, and
`attach_stdio_server` spawns it — before a model turn, before a permission ask, outside the
sandbox. It was the **one declaration that escaped the rule**, and it escaped because it is a
list of *servers* rather than a list of *roots*, so nobody had written the filter.

The trust record made it worse than an omission. `.epsilon/trust.json` is repository-writable
and pre-seeds the hash (A37), and first sight of a config is already `Trusted`
(`runtime/src/lib.rs:179-186`, `:4962-4966`) — so "wait for the prompt" was never a step an
attacker had to get past. **The fix therefore had to be at the spawn, not in the prompt.**

```rust
pub fn mcp_servers_trusted(user, project, project_trusted) -> Vec<McpServerConfig>
```

A pure function beside its two siblings, because the property worth pinning is the one an inline
`if` hides: *the project's servers disappear and the user's do not*. Verified by control —
removing the gate puts `curl … | sh` back in the untrusted list and the test fails naming it.

### 2. A repository could answer the trust prompt itself

`.agents/prompts` and `.epsilon/prompts` load at startup with no trust check, and the expanded
body goes back through `parse_command` — so a repo shipping a template whose body is `/trust yes`
permanently opens its own gate. `TrustRequiringResource::Prompts` has existed in the enum since
the day it was written and **nothing in production ever read it**: a declaration with no
consumer. That is the "faithful port, no production caller" shape, where the port *is* the
declaration.

Two things the ledger row did not carry:

- **The discovery call ran sixteen lines above the assignment of the flag it consults.** An
  ordering bug inside a security check survives review, because every line is correct alone.
- **`.agents/prompts` was never in the detector at all**, only `.epsilon/prompts` — so the
  cross-harness spelling this repository's own `.agents/skills` follows shipped templates no
  trust detector named.

### 3. `File::flush()` on a `File` is a no-op

```rust
self.file.write_all(line.as_bytes())?;
self.file.flush()?;          // ← looks like durability; is not
```

`std::fs::File` is unbuffered, so `Write::flush` has nothing to push. Everything the README
promises about a resumable session rested on the OS page cache surviving a power cut. Hermes
fsyncs for the same reason and says why in one line (`batch_runner.py:295`: *"so a crash never
loses an acknowledged prompt"*). One line, and it is `sync_data` rather than `sync_all` so it is
`fdatasync` rather than a metadata write.

---

## The class that turned out to be a habit

Scanning tracked files for bytes no gate can name turned up **fifteen** C0 control characters
across eleven files, and every one was the same corruption: a letter replaced by the C0
character of its own C escape name.

```
crates\epsilon-runtime\src\text_safety.rs:3   <0x1B>psilon-tui
crates\epsilon-cli\src\main.rs:207             <0x1B>psilon-runtime with <0x08>uild_runtime
crates\epsilon-core\src\lib.rs:575             <0x1B>xternal_dir
docs\milestones\M209-…md:46                    <0x0C>mt --check
```

**Two of the eleven were inside `epsilon_runtime::text_safety`** — the module whose entire
purpose is stripping exactly these characters. Every gate was green: `fmt` reprints text,
`clippy` lints code, the 1,600 tests assert values, and `check-line-endings` looks for CRLF.
A control byte inside a doc comment is none of those things.

`scripts/check-control-bytes.py` is the gate. It prints the line with the byte rendered as
`<0x1B>`, which is what makes it findable — and **it found a sixteenth instance on its first
run**, a form feed where `f` should have been, in a milestone log.

**Verified by control:** one byte injected into `scripts/check-links.py` produced
`check-links.py:3 byte 0x1B |Why this is a gate and not a one-<0x1B>off repair|`, and the file
was restored byte-identically (`git hash-object` equal before and after).

---

## The screen that said nothing

The permission dialog is the highest-stakes surface in the application: a reader is deciding
whether work runs. `handle_approval_key` ended in `_ => {}`, so Enter, Tab, Space, the arrows,
Home/End and Delete **did nothing and printed nothing**. G10's notice — this repository's own
"nothing happened must never be a reachable state" rule — lives at the end of `handle_key`,
which that function returns before ever reaching.

Guidance mode was worse: it is a *text field*, and it bound `Enter`/`Esc`/`Backspace`/`Char` and
nothing else, so the entire editing vocabulary was dead on the screen where a tool call is
blocked. And `guidance_mode` itself was **set, handled, and drawn nowhere** (A42) — the dialog
kept advertising `y allow once` while `y` typed a letter into the composer.

Pressing `e` also **destroyed the composer's draft** with no undo entry and no notice. It now
`stash_draft`s, and `esc` unstashes — the existing mechanism for exactly this borrow (a dialog
taking the editor over), not a second way of holding a draft.

The caret went with it: `overlay_open()` includes `pending_ask`, which is right for the picker
and the palette and wrong here, so guidance mode was the only state in the application where you
typed into a field with **no cursor in it**.

---

## The panic

```rust
let display = if raw.len() > budget {          // BYTES vs COLUMNS
    format!("{}...", &raw[..budget - 3])       // BYTE slice
```

The prefix guarantees the skew — `📁` is four bytes, the fold marks three — and snippets are
produced up to 100 columns wide, so at an 80-column terminal this branch is the **normal** path,
not an edge case. Any em dash, curly quote or CJK glyph within three bytes of the cut ended the
process with `byte index N is not a char boundary`.

The CJK test that already existed exercises `fit_width` for the *search field*, not this row, and
its cut landed on a boundary by arithmetic luck. One line, through the helper the file already
uses two functions below for the same budget.

**The control is the panic itself**: with the old slice restored, the new test fails with a
`byte index … is not a char boundary` rather than an assertion message. There is no build on
which the old line survives the fixture.

---

## Glyphs (A43), and why the row understated itself

`📁`, `▾`, `▸` were string literals in the timeline panel. The ledger called that a repertoire
gap; it is worse than that, because **ASCII is the *default* on a Windows box without a UTF-8
host** (`theme.rs:429-433`) — so most of the people filing the ASCII reports were seeing boxes
where the **two glyphs that distinguish collapsed from expanded** should be. That is the entire
purpose of the fold marker, and a box destroys exactly the difference it exists to carry.

The CP437 frame test never drove `draw_timeline`, which is why the M219 sweep missed them. Two
more were found while fixing them: `⇧L` in the hint row (the *only* place the `Shift+L` chord is
advertised) and the blockquote `❯` — which is in **model output**, the densest region of the app.

Five new fields, added to `Glyphs::ALL`, `SINGLE_GLYPH_FIELDS` and `named()` so the
exhaustiveness check covers them.

---

## Key decisions

- **The MCP gate is a pure function, not an inline `if`.** Beside `skill_roots_trusted` and
  `extension_roots_trusted`, with the same shape, because the property an inline conditional
  hides is the one worth pinning: *the project's servers disappear and the user's do not.*
- **`sync_data`, not `sync_all`.** It is `fdatasync`, which skips the inode metadata an append
  mostly does not need. The trade is a few milliseconds per append on Windows, paid only when an
  entry is written — which is why it lives here rather than in a flush a caller must remember.
- **A control byte is a gate, not a habit.** It costs one second and it names the file, the line
  and the corrupted word.
- **Guidance mode replaces the answer keys rather than joining them.** Printing the guidance
  line *underneath* `y allow once` would still leave the screen advertising two keyboards at
  once — the same lie with an extra sentence.
- **`hint_row` gained a width-budgeted sibling, `hint_row_in`,** rather than a signature
  change: ten other callers exist, and churning them all to fix one panel is not a smaller diff.
  Hints drop last-to-first and the count of what was dropped is drawn, because a silently
  shortened hint row *is* the discoverability failure the hint row exists to prevent.
- **The control-byte gate imports its file-selection policy** from `check-line-endings.py`
  rather than restating it. Two lists answering the same question is exactly the duplication
  ledger A12 records as this repository's own D-class failure.

## What nothing exercises

- **No frame test drives `/tree` in the ASCII repertoire.** The five new glyphs are in the table
  and the `named()` arm, and `an_ascii_frame_draws_no_character_a_legacy_console_cannot` does not
  paint the timeline panel — which is the reason the original four went unnoticed. The glyphs
  are now *correct by construction*; the panel is still uncovered.
- **No test paints the permission dialog at two widths.** The guidance lines are asserted on one
  frame at 100×30.
- **`mcp_servers_trusted` is tested as a pure function.** No test starts a real MCP server, so
  nothing proves the *call site* passes `project_trusted` rather than a literal — which is the
  same shape as every "faithful port, no production caller" row in the ledger.
- **The `.pre-epsilon` backup ordering defect (found, not fixed)** is recorded in the ledger
  rather than fixed here: `append` writes the header onto a headerless v1 file *before*
  snapshotting it, and the handle is `O_APPEND`, so the backup is a file this build refuses to
  open with `SecondHeader`.

---

## Batch two — the three security rows the sweep named

Second pass, same session. These are the rows a security reviewer **confirmed against the
code** rather than inferred.

### A33 — `.env.` is not `.env`, and Win32 opens it as one

`is_sensitive` compares `file_name()` to an exact list; the rules are `^.*\.env$` and
`^.*\.env\..*$`. Neither matches `.env.`, `.env ` or `.env::$DATA` — and on Windows all three
**open** `.env`: Win32 strips a trailing dot and a trailing space, and `::$DATA` names the
unnamed alternate data stream. Every one fell through to `Rule::new("read", "*", Allow)`. The
whole defence for the model-facing tools is those rules, and each was bypassable by appending
one character.

Fixed with **one rejection in `paths.rs`**, above policy evaluation and above any syscall, so
it covers `is_sensitive`, `gate_subject` and the `@` expander at once. `Resolution` grew a
`deceptive` field so the refusal can say *what it is* — "resolves outside the workspace" on a
name the user can see is a sentence they will not believe, which is the failure `escaped_to()`
was introduced to fix one level up.

**Windows-only, and that is the correct scope rather than a convenience.** On POSIX `.env.` is
a genuinely different file; refusing it would break a legitimate path for no security gain. The
rule follows the behaviour of the *open*.

### A34 — the two children that ran third-party code with the whole environment

`shell`, `goal_gate`, `sandbox`, `jobobject` and the LSP path all scrubbed. `epsilon-ext` and
`epsilon-mcp` did not — and those are exactly the two that spawn a binary **from a
configuration file**. Both are leaves that cannot reach `epsilon-safety`, and "cannot reach"
reads as "does not apply" unless the harness hands the rule down. Both now take an injected
`ScrubEnv`, the arrangement `epsilon-lsp` already used; one `runtime::scrub_env()` serves all
three.

**The real fix is the guard.** The stdin-inheritance defect was fixed once, found on three more
sites, and *then* given a mechanical test — and credential scrubbing had no such test, which is
why this recurred through several batteries with everything green.
`no_production_spawn_hands_the_developers_credentials_to_a_child` walks every `Command::new`
under `crates/*/src` and requires each to reach the scrubber or write its own environment. It
fired on **four** sites on its first run; all four are exempt with the reason written down (the
user's own editor, the OAuth browser launcher, a junction fixture in `test_support.rs`).

> **The first version of that guard could not fail.** It accepted a bare `scrub_env` token in
> the window — and the *parameter* is still named `scrub_env` after the body that uses it is
> deleted. The control (remove the scrub call, expect red) came back **green**. Fixed to
> require a call (`scrub(&mut`, `.env(`, `scrub_secret_env`), re-run, and the control now fails
> naming the exact line. This is the repository's own rule paying out on code written under that
> rule: *a control that passes is a control you have not proven.*

### A36 — a goal gate is shell, and nobody was asked

Gates arrive in the `gates:` argument of an `update_goal` tool call, so **the model writes
them**. They ran under the tool's own name; `gate_subject("goal", …)` yields the capability
`goal`, which no rule in the default ruleset mentions — so `Rule::new("bash", "*", Ask)` was
never consulted. One "allow once" on some unrelated bash call, and any number of arbitrary
shell commands, with a dialog the user had never been shown.

The sandbox axis was always honest (a gate shares `bash`'s sandbox and fails closed without
one), so this was an **authorization** bypass rather than a sandbox escape. What was missing
was the question. Gates now emit a `PermissionRequest` with capability **`bash`**, putting them
back under the rule the user already agreed to answer and under any project `[[rules]]` layer
they wrote for shell — and the ask sits *above* the unconfined branch, because `--no-sandbox` is
containment and this is consent.

**A runner with no approver wired now refuses**, rather than running. That is the difference
between "nobody thought about it" and "nobody wired it": the first is a hole, the second is a
report the goal reads and can act on.

## Verification

**Two batteries were run; both stopped at the `test` leg for the same environmental reason.**
Everything else is green, and the suite was measured per crate instead. Current numbers are
in [`docs/STATUS.md`](../../docs/STATUS.md); the detail is here.

| Gate | Result |
|---|---|
| `check-links` | PASS — 673 relative links, 0 broken |
| `check-lint-tables` | PASS — 13 crates inherit the workspace, 1 hand-copied and matching |
| `check-line-endings` | PASS |
| `check-control-bytes` (new) | PASS — 932 tracked files, 0 control bytes |
| `check-agents-lock` | PASS — `AGENTS.md` matches its SHA-256 |
| `fmt --check` | PASS |
| `clippy -D warnings` | PASS, zero warnings |
| **`test`** | **DID NOT RUN AS ONE COMMAND — see below** |
| `demo` | PASS |
| `real-check` | PASS |

### The `test` gate, and what was actually measured

Avira quarantines a freshly linked test executable mid-build. It is the fault this repository
has documented since M217, and it recurred on **both** the pinned path
(`epsilon_core-4a8081d0361ce74c.exe`) and, after the documented remedy, on the renamed binary
too. The full-workspace `test` leg fails with `LNK1104: cannot open file …`; where the binary
survived linking it failed at exec with `Access is denied. (os error 5)` — *never executed*.

**The documented remedy was applied and did not clear it.** `CARGO_PROFILE_TEST_DEBUG=1`
forces a full test-profile rebuild with fresh metadata; Avira caught the renamed binary. AGENTS.md
is explicit that changing security configuration is an owner decision, so this was not touched.

**So the suite was run crate by crate, which avoids the link burst, against the same bytes:**

| crate | passed | | crate | passed |
|---|---|---|---|---|
| `epsilon-contracts` | 82 | | `epsilon-mcp` | 6 |
| `epsilon-llm` | 104 | | `epsilon-ext` | 40 |
| `epsilon-session` | 114 | | `epsilon-runtime` | 97 |
| `epsilon-core` | 314 | | `epsilon-tui` | 506 |
| `epsilon-safety` | 55 | | `epsilon-server` | 16 |
| `epsilon-tools` | 205 | | `epsilon-cli` | 127 |
| `epsilon-lsp` | 25 | | `epsilon-desktop` | 161 |

**1,852 tests passed, 0 failed, across all 14 crates.** Every one was green; none was skipped,
filtered or ignored except three pre-existing `#[ignore]`s in the desktop crate.

**Three failures were found and fixed this way**, and the battery alone would not have found them
in a form anyone could read:

- `the_tree_panel_draws_its_filter_search_field_and_fold_glyphs` **failed**, and it was right
  to. Its assertions named `▾` and `▸` as *literals* — the very defect A43 records. Routing the
  panel through the repertoire table made the test see what a user sees on an ASCII console. It
  now asserts the two states get two **different** glyphs **per row**, plus the ASCII repertoire
  itself, which the literal form could not express at all. **A test can be an instance of the
  bug it is next to.**
- `LNK1318: Unexpected PDB error; LIMIT (12)` on the desktop link — a stale MSVC PDB server,
  cleared by stopping `mspdbbsrv` and deleting the 421 `.pdb` files in the build tree. New on
  this machine; recorded in [`05-machine.md`](../../agents/memory/05-machine.md).

### Limits

- **No single battery run covers these bytes.** `audit/evidence/gate-m276i.log` is a `GATE FAIL`
  with the `test` leg at exit 101. The per-crate numbers above are a real measurement of the
  same tree, and the distinction is the point: *a battery that did not run is not a pass.*
- **The failure was never reproduced against a specific test.** Every individual crate is green,
  which is the evidence that it is the build, not the code — but "the link burst trips a
  heuristic scanner" is a diagnosis from the error text, not a measurement.
- **`epsilon-desktop` and `epsilon-tui` were each run twice** — once failing, once green — after
  edits made in response. The numbers above are the second run.
- **Nothing was reinstalled.** `cargo install` was not run, so the owner's global `epsilon.exe`
  does **not** carry these changes. The changes are CLI/TUI-visible (the permission dialog, the
  tree panel's glyphs and truncation, the footer), so a reinstall is owed before the installed
  binary is the thing being tested.