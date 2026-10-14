# Permissions and safety

For users deciding what execution they can safely permit. Permission rules apply
to tools; OS confinement applies only to supported subprocess paths, not every
native filesystem operation.

## Layer 1 — permission rules

Rules are `{capability, pattern, action}` entries with wildcards. **Any matching
deny wins**; otherwise the later matching allow/ask rule determines the result.
`action` is allow, ask, or deny. Interactive runs show an approval prompt for `ask`; the
project ruleset lives in the trust-gated `.epsilon/config.toml` (see
[`configuration.md`](configuration.md)).

Subjects are lexically normalized, so `.env`, `./.env` and `src/../.env` are the same rule
target. Paths outside the workspace are gated as `external_dir` regardless of the tool name.

Headless behavior: `-p` / `--serve` runs **auto-deny** `ask` rules. `--yes` flips that to
auto-approve — an explicit opt-in for trusted automation.

`--plan` adds deny rules for the native `write`, `edit`, `bash` and `shell`
capabilities while retaining project `[[rules]]`. Reads and search remain subject
to secret-file, external-path and project permissions. Internal session bookkeeping
still writes, and these rules do not classify the side effects of arbitrary
extension or MCP tools.

**Post-fix state:** the original tool capability is now always authorized before
any path-derived `external_dir` authorization, and either denial short-circuits
(`crates/epsilon-core/src/exec.rs:337-391`). A path-ish argument can no longer
mask a plan deny on `bash`/`shell`. Remaining limits: plan mode does not
classify the side effects of arbitrary extension/MCP tools, and out-of-workspace
subjects keep their raw spelling (known prompt-dedup limitation, no bypass).

## Permission presets

Four named postures (ADR-032). A preset is a bundle of allows, never a pardon:
project and user deny rules layer above every preset and win, and switching
presets clears the session's always-allow memory.

```sh
--preset auto                  # today's behavior under a name (default)
--preset workspace-write       # reads and writes under the workspace; asks outside it
--preset read-only              # denies write/edit/bash/shell; reads stay free
--preset danger-full-access     # allows everything nothing forbids
```

`/preset` lists the four with the current marked and switches mid-session in
the terminal IDE. Switching *down* applies at once; switching *up* to
`danger-full-access` asks first — type `yes` to confirm, anything else leaves
the posture unchanged. Parked approval asks keep the policy that raised them.

## Layer 2 — OS confinement for shell commands

```sh
--sandbox auto        # default: best backend, fail closed when none exists
--sandbox read-only   # commands may look, not touch (workspace mounted read-only
                      # where the backend supports it)
--sandbox off         # no confinement at all — trusted automation only; warns loudly
```

Backends: **Job Objects** on Windows, **bubblewrap** on Linux.

Honest limits (evidence in [`../../audit/03-defects.md`](../../audit/03-defects.md)):

- Windows Job Object confinement is **partial** relative to bwrap: it bounds process
  resources and process ownership, but does not restrict filesystem or network access.
  A read-only mode label is not an OS-enforced Windows write restriction. See
  [the architecture guide](../design/epsilon-architecture.md#tools-and-execution-boundaries)
  for the implemented boundaries.
- Path containment for the file tools is lexical; symlink/alias traversal is a **known open
  defect (F09)**, not a guaranteed boundary. Treat workspace trust accordingly.
- macOS has no confinement backend: with `--sandbox auto` on macOS, shell commands fail
  closed (refuse to run) rather than run unconfined.
- Process-isolation gaps (descendant processes outliving cancellation) are tracked as
  F11/F12 in the ledger.

## Credential hygiene

Shell execution filters credential-shaped environment variables. This is not a
secret-isolation guarantee: native reads, readable host files, subprocess-specific
environments and unrecognized variable names need separate controls.

Displayed shell output is capped, but the current parent collector buffers complete
output first (F12). Spill-file availability does not establish bounded live memory.
Timeout/cancellation and inherited-pipe cleanup gaps are tracked in F11.

## Secret files: what is refused, and what is not

Two ways to get a file's contents in front of the model behave differently, and the
difference is worth knowing before you paste something you would rather not keep.

| | `@.env` (a reference) | `read .env` (the tool) |
|---|---|---|
| refused? | **yes** — by name, before anything is read | no, unless a permission rule denies it |
| what you see | the file is folded into the prompt; its contents do **not** appear in the transcript | the contents appear as a tool result, in front of you |

The refusal is by **name**, over a fixed list (`is_sensitive`): `.env`, `auth.json`,
`credentials`, `id_rsa`, `id_ed25519`, `.netrc`, `.npmrc`, `.pgpass`, `secrets.json` and a
few more. It is checked against both the path as spelled and the path it resolves to, so a
symlink or junction pointing at one of those names is refused too — that check was added
after a link was found to walk an innocent name past it.

The asymmetry is the point rather than an oversight: an `@`-reference attaches a file
*silently*, so a credential that goes that way reaches every export, summary and log
downstream without ever having been on screen. `read` shows you what the model got. Neither
is a guarantee — if you ask the model to read a secret, it will, and the contents land in
the session file.

**The session file is not scrubbed.** Nothing filters credential-shaped strings out of a
tool result before it is appended to the session JSONL, so a `cat .env` through `bash`
stays in that file in cleartext, along with anything exported from it. Shell execution does
filter credential-shaped *environment variables* (above), which is a different control over
a different surface. Whether the durable store should scrub, and how, is an open product
decision rather than a missing feature — the trade is that scrubbing keeps a secret out of
an on-disk transcript and also hides evidence from you, and a pattern that is wrong either
misses a secret or mangles a file's contents. Until that is decided, treat the session file
as you would treat your shell history.
