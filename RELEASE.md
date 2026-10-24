# Epsilon 0.1.0 — release candidate

**Status:** release candidate. The credential model deliberately follows Pi
1.3.0; this is not a claim of production-grade secret storage. The release needs
a reviewed commit series and the macOS CI result before it becomes final.

## Verified build

- Full battery: **14/14 legs green** — `target/gate-final-audit-7.log`.
- Workspace suite: **2,658 passed / 0 failed / 21 ignored**, 77 test binaries.
- Terminal-IDE suite: **918 passed / 0 failed / 20 ignored**.
- Cache wire tests: **7/7 passed** under `cargo test -p epsilon-llm --features real`.
- Documentation links: **300 checked, 0 broken**.
- Installed binary: `epsilon 0.1.0`, built and installed on Windows on
  2026-10-24 at 17:04 local; `--demo` completed and REA attached with 207 tools.

The battery was run with the host's installed `1.97.1` Rust toolchain and the
Visual Studio 2019 Build Tools environment because this host's stable rustup
toolchain and VS2022 linker were incomplete. That is an environment note, not a
project pin.

## Install

Windows:

```powershell
powershell -ExecutionPolicy Bypass -File .\install.ps1
```

macOS / Linux from source:

```sh
bash install.sh
```

`install.sh` has not been verified on a macOS host in this audit. The Windows
installer was run on this host. The macOS CI job is defined in
`.github/workflows/ci.yml` but has not executed.

## Credential model: Pi parity

Epsilon stores provider credentials in `<config dir>/auth.json` in plaintext:
directory mode 0700, file mode 0600 on Unix, atomic replacement, a warning at
login and list. **This is the same model Pi 1.3.0 ships**:
`references/pi-1.3.0/packages/coding-agent/src/core/auth-storage.ts` writes
`auth.json` with mode `0o600` and creates the agent directory with mode `0o700`.
Pi does not use an OS keyring for this file either. Epsilon is stricter in one
respect: it writes through a temporary file and rename rather than truncating in
place.

That makes plaintext storage an intentional **reference-parity decision**, not a
missing port. It is still a real security limitation:

- Prefer environment variables for CI and shared machines.
- Keep the file owner-only; treat the config directory as sensitive.
- An OS-keyring backend remains optional hardening, not a parity blocker. If it
  is added later it needs a new ADR, a migration path and its own tests.

## Cache measurements are wire-level

The prompt-cache fixes are verified against the actual request bodies:
Anthropic tail marker placement, grouped tool results, and OpenAI/Mistral
content-derived affinity keys. No paid provider call was made, so no live
cache-hit rate or token-saving percentage is claimed.

## macOS is build-tested only in configuration

The macOS CI job is present and its YAML parses, but it has not run. macOS builds
from source by report, has no native shell-confinement backend, and should be
expected to refuse side-effecting shell calls by default. The unrun job is the
main remaining release-candidate caveat.

## Documentation scope

The documentation link graph, command count, skill count, ADR numbering and the
major contradictions were checked and corrected. The audit did not read every
sentence of every Markdown file line by line; `docs/04-STATUS.md` remains the
moving record.

## Deliberate design differences

- OpenAI/Mistral cache keys are content-derived (model + system prompt + first
  user text), not Pi's literal session-id key. They are stable and sent only to
  exact first-party hosts.
- The explorer polls: a 900 ms tree stamp and a 3 s `git status`. It has no
  filesystem-watcher dependency. `git` is resolved by absolute PATH and
  `core.fsmonitor` is disabled so a workspace cannot choose the binary or run a
  hook through the poll.

## Before final publication

1. Run the macOS CI job and record the installer result.
2. Review and split the worktree into a reviewed commit series; this audit made
   no commit.
3. Measure cache-hit rates against a paid provider before claiming token savings.
4. Treat an OS-keyring backend as optional follow-up hardening, not a release
   blocker for Pi parity.

The permanent engineering rules are in [`AGENTS.md`](AGENTS.md). Current status
and the defect ledger are in [`docs/04-STATUS.md`](docs/04-STATUS.md).
