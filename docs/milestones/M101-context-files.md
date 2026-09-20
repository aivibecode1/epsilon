# M101 — The context files the harness was already detecting

Date: 2026-09-20. Trigger: `audit/evidence/port-fidelity-audit.md` findings D-7
and D-8, plus a third gap the same audit listed (D-9's remainder). Two of the
three were real; the audit was wrong about one and understated another.

## What was wrong

Three separate ways the harness told the user something that was not true.

**A trust prompt for a file that never loaded.** `.epsilon/SYSTEM.md`,
`.epsilon/APPEND_SYSTEM.md` and `.epsilon/prompts/` were all listed as
trust-requiring resources — so writing one raised a prompt — and none was ever
read. A user writing `SYSTEM.md` answered a trust question and got no effect.
That is worse than not supporting the feature: the prompt implies it worked.

**`AGENTS.override.md` missing from the candidate list**, and worse, `doctor`
carried a *second, shorter* list of its own. A user with an override was told by
the diagnostic that no instructions were loaded.

**Ancestor context files never read.** pi walks to the filesystem root; we loaded
the workspace only, so a parent repository's `AGENTS.md` had no effect on a
workspace nested inside it.

## What changed

### The overlay loads, and replaces the contract only

`epsilon-core::prompt` gains `SystemOverlay` and `load_system_overlay{,_in}`,
mirroring pi's `discoverSystemPromptFile` / `discoverAppendSystemPromptFile`
(`resource-loader.ts:1023-1049`). The project pair shadows the global pair rather
than adding to it, matching pi. A blank file counts as absent, also matching pi.

**One deliberate divergence, recorded here because it is the kind of thing a
later reader would otherwise "fix" back:** a custom identity replaces the
*contract* only. The generated `# Available tools` and `# Guidelines` sections
stay, because they describe the live registry and a stale hand-written list of
tools is a lie the harness would be telling about itself. pi drops its own list
along with the default prompt (`system-prompt.ts:48-73`); we keep ours on
purpose.

### `.epsilon/prompts/` stays detected and unloaded

This is the correction to the audit, and it is a correction to *this milestone's
own first commit message*. `.epsilon/prompts/` is not system-prompt text: pi's
`prompts/*.md` are user-invoked slash templates with `$1` / `$ARGUMENTS`
substitution (`prompt-templates.ts:11-18,70-80`), and the template feature is
unported. Loading them into the system prompt would be wrong, so the trust gate
stays in place — it is what stops untrusted templates loading on the day the
feature lands.

### One context file per directory, and the ancestor walk

`CONTEXT_FILE_CANDIDATES` is pi's five names. The important part is the
semantics, not the list: **one file per directory, first existing wins.**

The audit's "smallest fix: add the two names" would have been wrong. The old loop
loaded every candidate it matched, so simply adding `AGENTS.override.md` would
have made the override *additive* — both files in the prompt, the override no
longer overriding. The precedence that now holds, lowest to highest:

1. global `~/.config/epsilon/MEMORY.md`
2. ancestor context files, root first
3. the workspace's own context file (`AGENTS.override.md` beating `AGENTS.md`)
4. workspace `MEMORY.md`
5. M73 touched-subdirectory hints

Ancestor files are labelled by absolute path — a relative `../AGENTS.md` per
level names no usable location — while files inside the workspace keep their
short relative labels. The M73 hints loader uses the same candidate list, so an
override works in a subdirectory too, and `doctor` reads that same list instead
of keeping a second copy.

### The skill catalog was already correct

D-9's remainder asked whether `disable-model-invocation` is filtered everywhere
it matters. It is: `skills::catalog` filters it and is the only prompt path; the
`skill` tool is built from an explicitly filtered `Vec<LoadableSkill>`, a type
with no disabled flag, so the filter cannot be skipped by accident. `/skills` is
user-facing and *should* list disabled skills. Nothing changed, which is the
right outcome for a gap that turned out not to exist.

## Verification

- `cargo test --workspace` — **975 passed, 0 failed, 1 ignored** across 57 binaries
- `cargo test -p epsilon-core` 242/0; `cargo test -p epsilon-cli` 58/0
- `cargo clippy --workspace --all-targets -- -D warnings` exit 0
- `cargo fmt --all -- --check` exit 0
- `cargo check -p epsilon-cli --features real` exit 0
- `cargo run -q -p epsilon-cli -- --demo` exit 0 (`final reason: Completed`)
- New tests: 16 in `prompt.rs` (override precedence, ancestor order, the overlay
  loader including trust and global fallback, contract replacement, append
  position, fingerprint), 1 in `epsilon-cli/src/runtime.rs` asserting every
  trust-gated `SYSTEM.md` name is actually loaded, 1 hint test, and 3 in
  `epsilon-core/tests/system_overlay.rs`.
- **Negative control**, which is the evidence that matters: each fix reverted in
  place, restored byte-identically (`sha256 52D19BD9…`), and 6 prompt tests plus
  2 of 3 integration tests plus the fingerprint test failed for their own named
  reason. Logs: `audit/evidence/g7g8-negative-control-*.log`.

## Limits

- `.epsilon/prompts/` remains unported, as above.
- The global `~/.config/epsilon/AGENTS.md` family — pi's agentDir context file —
  is still not loaded. Out of scope here and now recorded rather than implied.
- The ancestor walk stats every directory from the workspace to the filesystem
  root on each prompt build. Cheap for the depths real repositories have; it
  would need a cache before it is cheap for a path near the root.
