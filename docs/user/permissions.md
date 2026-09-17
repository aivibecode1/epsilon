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

## Layer 2 — OS confinement for shell commands

```sh
--sandbox auto        # default: best backend, fail closed when none exists
--sandbox read-only   # commands may look, not touch (workspace mounted read-only
                      # where the backend supports it)
--sandbox off         # no confinement at all — trusted automation only; warns loudly
```

Backends: **Job Objects** on Windows, **bubblewrap** on Linux.

Honest limits (evidence in [`../../audit/02-defect-ledger.md`](../../audit/02-defect-ledger.md)):

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
