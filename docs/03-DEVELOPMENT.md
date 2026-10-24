# Development

**Read this before your first change.** It covers the build, the test battery, the
machine rules, and where a change belongs. The permanent contract is
[`../AGENTS.md`](../AGENTS.md); the current state and limitations are
[`04-STATUS.md`](04-STATUS.md).

Read in this order:

1. [`../AGENTS.md`](../AGENTS.md) — the contract. Read it in full, every session.
2. **This file** — how to build, test and recover.
3. [`04-STATUS.md`](04-STATUS.md) — what is true now, and what is still limited.
4. [`02-ARCHITECTURE.md`](02-ARCHITECTURE.md) — the crate map and execution paths.

## 1. The shape in three sentences

Epsilon is a coding-agent harness in Rust: an agent core that reads, edits, runs
commands and keeps a resumable session. It has **one terminal IDE over one core** — a
full-screen app (`epsilon-cli` / `epsilon-tui`) plus headless, HTTP/SSE and stdio-RPC
modes. `epsilon-runtime` is the only place a workspace plus a configuration becomes a
running agent (`build_runtime`, `resolve_launch`); a second copy of composition is a
defect, not an optimisation.

## 2. Build and test

Toolchain: **Rust 1.88+**. Every crate inherits the workspace `rust-version`; no
manifest overrides it.

```sh
cargo check --workspace --all-targets
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p epsilon-llm --features real   # provider wire + cache-breakpoint tests
cargo run -q -p epsilon-cli -- --demo      # offline, scripted, no provider needed
cargo check -p epsilon-cli --features real
cargo run -q -p epsilon-cli                # the terminal UI
```

`clippy` is zero-warning with no exceptions. `cargo test --workspace` is the suite;
nothing hits a paid API unless a test is explicitly enabled.

### The battery

The full battery runs the repository checks, `fmt`, `clippy`, the workspace tests, the
offline demo and the real-feature build — fourteen legs in all:

```sh
pwsh -File scripts/gate.ps1 <label>    # Windows — use this one here
bash scripts/gate.sh <label>           # POSIX / CI
pwsh -File scripts/gate-until-pass.ps1 -Label <label>   # retry with a fingerprint
```

The log lands at `target/gate-<label>.log`. **Never re-use a label**: `gate.ps1`
truncates the log, so a reused label destroys the failed attempt's evidence. **Do not
edit source while a battery runs** — the log then describes a tree that no longer exists.
Give the command a timeout above 600 s; the test leg alone takes several minutes.

## 3. This machine

- **One build owner at a time.** A battery, a delegated build and a `cargo install` must
  not share the machine. Use `CARGO_BUILD_JOBS=1`; parallel jobs have failed here with
  `0xc0000142` compiler-startup contention.
- **Build outside the repository.** `CARGO_TARGET_DIR` goes to `%TEMP%\epsilon-target`.
  Build output inside this repository makes `cl.exe` fail with `D8050: cannot execute
  c1.dll`, which cargo reports as a broken *dependency*. Any shell build must export the
  environment first:

  ```sh
  $env:CARGO_TARGET_DIR = "$env:TEMP\epsilon-target"
  $env:CARGO_BUILD_JOBS = "1"
  $env:CARGO_PROFILE_TEST_DEBUG = "0"
  $env:CARGO_PROFILE_DEV_DEBUG = "0"
  ```

- **Disk is tight.** Debug info off runs the full battery in ~4 GB instead of ~33 GB.
  Turn it back on for the round that debugs a failure, and name the knob wherever you
  cite the battery.
- **`LNK1104` on a fresh test executable is a machine fault, not a defect.** Avira can
  flag a path and the link fails on a file that does not exist. Rebuild the test profile
  under fresh metadata (`CARGO_PROFILE_TEST_DEBUG=1`) and disclose the knob. Never change
  security configuration from an agent session.
- `bash` here is WSL and may not have a working Windows `cargo`. Prefer `gate.ps1`.
- The repository is **LF** in the index and on disk (`.gitattributes`:
  `* text=auto eol=lf`).

## 4. Where a change belongs

```text
crates/
  epsilon-contracts   shared types and traits; zero internal dependencies
  epsilon-llm         provider protocols, faux provider, routing, catalog, auth, retry
  epsilon-session     JSONL storage, branching, leases, repair
  epsilon-core        the agent loop, scheduling, context assembly, compaction, tasks
  epsilon-tools       native tools, shell adapter, checkpoints, goals
  epsilon-safety      permission engine, approvals, sandbox backends
  epsilon-lsp         optional language-server client and diagnostics
  epsilon-ext         process-extension protocol and host
  epsilon-mcp         MCP client and tool adapter
  epsilon-runtime     COMPOSITION: the one place configuration becomes an agent
  epsilon-tui         terminal IDE: tree, viewer/editor, transcript, palette
  epsilon-server      HTTP/SSE frontend
  epsilon-cli         the binary: argument parsing and mode dispatch
```

- **A capability belongs in the core**, not in one front end. The terminal, headless,
  server and RPC modes are modes on the same loop.
- **A command belongs in `epsilon-contracts`** (`COMMAND_HEADS` + `SLASH_ITEMS`), read
  by every mode. The palette derives from the same table; a second hand-written list is
  the defect `AGENTS.md` §2.4 forbids.
- **A UI sees the frame.** For anything a user sees, render it and read the screen at
  80×24 and 100×30 — a state assertion is not a claim about what is drawn.
- **A settings write is user work.** Read-modify-write must refuse a present-but-
  unparsable file and publish with a staged write + rename; use
  `epsilon_core::config::read_config_table` / `write_config_atomic`.

## 5. Evidence discipline

These rules exist because each failure has cost this repository real time:

- **A passing test is not a true claim.** Ask what the broken version would print and
  confirm the difference.
- **Grep for callers, then check at least one is production code.** A definition plus a
  test is not a feature.
- **Measure the number the code reads**, not the one you reasoned about.
- **A frame is not asserted until you have looked at it.**
- **Never quote a test count you did not measure.** Cite the log and the command.

## 6. Security and data loss

- Every side-effecting tool call passes the permission gate. Deny is monotonic; an
  unknown capability is `Ask`, never `Allow`.
- The approval channel fails closed: a dropped responder resolves to deny.
- Untrusted text is stripped of control and bidi characters before it reaches a screen.
- Never lose a user's work: confirm before discarding, keep a recoverable copy, and make
  a refusal path leave the file untouched.
- Validate every boundary — paths, RPC frames, HTTP bodies, MCP payloads. A path crossing
  into the filesystem is normalised and checked for escape, not joined and trusted.
- A new dependency is a supply-chain decision. Direct dependencies are pinned exact in
  `[workspace.dependencies]`; a new crate needs an ADR.

## 7. Recovering from a failed battery

1. Read the failing leg and the exact command in `target/gate-<label>.log`.
2. Reproduce that command alone with the environment above.
3. If the failure is a link error on a file that does not exist, treat it as the machine
   fault in §3 and re-run with fresh metadata.
4. Fix the cause, add a test that fails for the reason it names, then run the whole
   battery with a **new** label. Copy anything worth keeping out of `target/`.
5. Record what changed and what is still broken in
   [`04-STATUS.md`](04-STATUS.md).

## 8. Installing a build

Use the checked installer; it is the same path a user takes and it verifies the
binary after the build.

```powershell
powershell -ExecutionPolicy Bypass -File .\install.ps1
```

```sh
bash install.sh
```

Both run `cargo install --path crates/epsilon-cli --locked --force`, set
`CARGO_TARGET_DIR` outside the checkout when it is unset, and run
`epsilon --version` unless `--no-verify` is passed. `--prefix`/`-Prefix`
selects an install root. The manual `cargo install` form still works; the
installer adds the toolchain check, the out-of-tree target and the PATH
report.

Release builds are slower than the debug test profile and are a separate
compilation. Do not run one beside a battery on this machine (the one-build
rule in §3 applies to both).
