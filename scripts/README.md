# scripts

Verification and maintenance tooling. Nothing here ships: no script is read by a
compiled crate, and none is a build step. The installers are at the repository
root (`install.ps1`, `install.sh`), not here.

## The battery

Run the whole battery from the repository root. It is what CI runs and what
`AGENTS.md` means by "the battery".

```sh
pwsh -File scripts/gate.ps1 mylabel    # Windows — this machine's working path
bash scripts/gate.sh mylabel           # macOS, Linux, CI
```

The log lands at `target/gate-<label>.log`. `target/` is disposable: copy
anything you want to keep somewhere durable before clearing it, and record the
cited numbers in [`docs/04-STATUS.md`](../docs/04-STATUS.md).

| Script | What it does |
|---|---|
| `gate.ps1` | The fourteen-leg battery on Windows. Sets `CARGO_TARGET_DIR` and the MSVC environment itself, which is what makes it work on a bare Windows shell. |
| `gate.sh` | The same gates for POSIX and CI. It resolves `cargo` before running anything, because a WSL bash does not necessarily inherit the Windows `PATH`. |
| `gate-until-pass.ps1` | Wraps either gate. Gives every attempt its own label, fingerprints a failure (test names and machine-fault markers such as `LNK1104` or `0xc0000142`), and stops on the second identical fingerprint. It never edits the gate scripts. |

The order is: `check-links`, `check-lint-tables`,
`check-parity-citations`, `check-citation-placement`,
`check-reference-citations`, `check-verb-counts`, `check-parity-counts`,
`check-line-endings`, `check-control-bytes`, `fmt`, `clippy`, `test`, `demo`,
`real-check`.

**Two rules that cost real time when forgotten:** never re-use a label (the gate
truncates the log, so a reused label destroys a failed attempt's evidence), and
never edit source while a battery runs (the log then describes a tree that no
longer exists).

## Repository checks

Each can be run alone; all are fast.

| Script | What it fails on |
|---|---|
| `check-links.py` | A broken relative link anywhere in the documentation. |
| `check-line-endings.py` | A tracked text file carrying CRLF. |
| `check-control-bytes.py` | A C0 control byte inside a tracked text file. |
| `check-lint-tables.py` | A crate whose `[lints]` table neither inherits the workspace's nor matches it. |
| `check-reference-citations.py` | A `references/<project>/path:line` citation that does not resolve in the vendored tree. Needs `references/`, so it is skipped on a fresh clone. |
| `check-citation-placement.py` | A citation placed where the CI cannot see the file it names (for example, a test-only citation presented as production evidence). |
| `check-parity-citations.py` | A parity-matrix citation that does not resolve. |
| `check-parity-counts.py` | A parity page whose stated count disagrees with its rows. |
| `check-verb-counts.py` | A documented count of tools, commands or keybindings that no longer matches the registry. |

## Probes and maintenance

| Script | What it does |
|---|---|
| `console-probe.py` | Measures what the TUI does to a **real** Windows console window: opens a conhost, records geometry before and after, types real `KEY_EVENT` records, reads the screen back as JSON. Windows-only. |
| `rpc_probe.py` | Drives `epsilon --rpc` and reports frame by frame. For the wire, not the types. |
| `clear-cache.sh` | Reports or removes one build-cache directory. It refuses a path whose name does not contain `target`. |
| `regen_catalog.py` | Regenerates `crates/epsilon-llm/src/catalog_data.rs` from [models.dev](https://models.dev). The generated file is not hand-edited. |
| `remap-by-vintage.py`, `remap-citations.py` | One-off citation re-pointing helpers, kept because the shape of the breakage is worth remembering. |
| `report-citation-subjects.py`, `report-reference-fidelity.py` | Read-only inventories of what cites what, used when auditing a reference study. |
| `fix-links.py` | One-off repair for the 2026-09-28 documentation renumbering. |

## Conventions

Each check opens with a comment explaining why it exists, usually with the
incident that made it a gate. A check whose reason is forgotten gets deleted
when it becomes inconvenient; one whose reason is written down does not.
