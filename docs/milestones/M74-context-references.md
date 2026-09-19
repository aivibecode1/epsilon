# M74 — `@`-references: pointing at a file and getting the file

**Scope:** port-plan item 12 (Hermes `agent/context_references.py:212-484`).

## The problem

The TUI's `@` autocomplete inserted a *path*. The model then had to guess whether the file was
small enough to `read`, which of three similarly-named files was meant, or what part of a long file
mattered — and the guess cost a turn. Attaching the content removes the guess.

## What landed

`epsilon-core/src/references.rs`, used by **every** frontend through one constructor:

| Written | Attached |
|---|---|
| `@src/lib.rs` | the whole file |
| `@src/lib.rs:10-20` | lines 10–20 (1-based, inclusive, counted in the header) |
| `@src/lib.rs:10` | line 10 |
| `@crates/` | a sorted directory listing, sensitive entries omitted |

Wiring: the TUI's `submit_text`, the CLI's `-p`, the RPC loop and the HTTP server all build user
messages with `references::user_message(&workspace, text)`. A feature that works in one frontend and
not the others is a feature the others are missing — that was the audit's own finding about this
codebase, so it is not repeated here.

**Not expanded, deliberately:** `@diff`, `@staged`, `@git`, `@url`. Each needs either a confined
subprocess or a network fetch, and both live in the tools crate; the loop and the frontends must not
grow their own spawn path to satisfy a convenience (ADR-005's confinement rule). `bash git diff` and
`webfetch` already exist for the model.

## What it refuses, and why the refusal is visible

- **Sensitive paths** — `.env`, `id_rsa`, `*.pem`/`*.key`/`*.pfx`, anything named `*secret*` or
  `*credential*`, and everything under `.git`/`.ssh`/`.aws`/`.gnupg`. A credential file in the
  transcript is a credential in every export, summary and log that follows it. A refused reference
  says so in the message, and the assertion checks that **no content leaked with the refusal**.
- **Paths outside the workspace** — `@../x`, `@/etc/passwd`, `@sub/../../x`. Resolved with the same
  lexical normalizer the tools use, moved to `epsilon-core` this milestone so there is exactly one
  implementation and a file the tools call "inside" can never be one the expander calls "outside".
- **Binary files** — described (`a binary file (N bytes)`) rather than attached; the model should be
  the one to decide to read it.
- **Missing files** — reported. A silently dropped reference is how a user learns nothing happened.

## Key decisions

- **The user's words are never edited.** The message keeps the text exactly as typed — references
  stay visible in it — and the resolved content rides alongside as a separate `<user_attachments>`
  block. Rewriting the user's text would break the invariant that this harness never paraphrases a
  user turn, and would hide from the transcript what the user actually asked about.
- **A prompt with no references is byte-identical to before this feature** (`render()` returns
  `None`, one text block, asserted). A convenience feature must not change the common case.
- **An `@` inside a word is not a reference**, so `user@example.com` stays prose. This is the one
  rule that keeps the syntax from being surprising, and it has its own test.
- **The overflow is announced, not dropped** — more references than the budget (8) produces an
  explicit "ask again for the others", which is more useful than a silent truncation.
- **`resolve_in_workspace` moved to `epsilon-core::paths`** and `epsilon-tools` re-exports it, so the
  12 existing call sites (including M72's resource keys) were untouched: the rule moved, the call
  sites did not.
- The directory listing hides sensitive names rather than listing them with a refusal — the *name* of
  a credential file is itself information the prompt does not need.

## What the tests caught

The wiring, three times over: removing the top-level `new_id`/`UserMessage` imports for the moved
constructor left two test modules and one `tokio::spawn` broken — the borrow of `shared` cannot
outlive `serve_mode`, so the message is now built *before* the task is spawned (which is also the
right place: the expansion reads files). And `epsilon-tools`' `util.rs` kept using `PathBuf` after
the resolver moved, which the workspace check caught immediately.

## Verification

| Mutation | Result |
|---|---|
| sensitivity check disabled | 2 tests fail (the refusal and the listing) |
| workspace confinement ignored | the escape test fails |
| the word-start rule removed | the email test fails |
| attachments never attached | the message-shape test fails |

New tests: 14 in `references.rs` (file, ranges and their edges, listing, emails-vs-references,
sensitive paths with a leak check, escapes, missing, binary, the byte cap, the reference budget, the
no-reference case, message shape, and the range parser) and 1 TUI test asserting `submit_text`
attaches through the shared constructor and leaves a plain prompt alone.

Gate: `scripts/gate.sh m74` — fmt clean, clippy zero warnings, **689 tests, 0 failed**, demo
completes, `real`-feature build compiles.

## Plan status

Item 12 is **ported, minus the four spawn-or-fetch references** named above (recorded in
`03-port-plan.md` as the remaining piece). Tier 3 then has only item 13 (typed corruption reasons)
open.
