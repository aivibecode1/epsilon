# scripts

Verification and maintenance tooling. Nothing here ships: no script is read by the compiled
crates, and none is a build step.

**Run the whole battery from here.** It is the same command on every platform, and it is what CI
runs.

```sh
pwsh -File scripts/gate.ps1 mylabel    # Windows — use this one here
bash scripts/gate.sh mylabel           # macOS, Linux, CI
```

The log lands at `target/gate-<label>.log`. `target/` is disposable, so **copy anything you want
to keep out of it** — evidence files under `audit/evidence/` use a `.txt` extension for exactly
this reason (`*.log` there is gitignored).

## The battery

| | |
|---|---|
| `gate.ps1` | The full battery on Windows. Sets `CARGO_TARGET_DIR` and the MSVC environment itself, which is what makes it the only one of these that works on a bare Windows shell. |
| `gate.sh` | The same gates for POSIX and CI. |
| `gate-until-pass.ps1` | Wraps either. Gives **every attempt its own label**, fingerprints a failure (test names plus machine-fault markers like `LNK1104` or `0xc0000142`), and stops on the second identical fingerprint — which is what "not transient" means for a suite. It never edits the gate scripts; those are the evidence of record. |

Gates, in order: `check-links`, `check-lint-tables`, `check-line-endings`,
`check-control-bytes`, `fmt`, `clippy`, `test`, `demo`, `real-check`.

**Two rules that cost everyone time when forgotten:** never re-use a label (the gate truncates
the log, so a reused label destroys a failed attempt's evidence), and never edit source while a
battery is running (the log then describes a tree that no longer exists).

## Repository checks

Each of these can be run on its own; all of them are fast.

| Script | What it fails on |
|---|---|
| `check-links.py` | A broken relative link anywhere in the documentation. A dead citation looks exactly like a live one until a reader follows it. |
| `check-line-endings.py` | A tracked text file carrying CRLF. `.gitattributes` normalises on commit, so `git diff` shows a CRLF working copy as unchanged — and a CRLF working copy is a different file to every tool that parses this repository's source. |
| `check-control-bytes.py` | A C0 control byte inside a tracked text file. These are corrupted *letters*, not content (`e`→0x1B, `a`→0x07), which is why no other gate can see them: `fmt` and `clippy` read code, and a control byte in a comment is not code. |
| `check-lint-tables.py` | A crate whose `[lints]` table neither inherits the workspace's nor matches it. Cargo **replaces** rather than merges a package table, so a crate that must override one lint hand-copies the whole thing and silently misses anything added at the root afterwards. |
| `check-reference-citations.py` | A `references/pi-<version>/path:lines` citation that does not resolve against the vendored tree. **Not a battery gate** — it needs `references/`, which is not committed, so it cannot run on a fresh clone or in CI. Its version segment matches *any* pi version rather than one pinned release: pinned to `0.84.2` it silently ignored every other version, which reads as coverage while checking almost nothing. |

## Probes and repair

| Script | What it does |
|---|---|
| `console-probe.py` | Measures what the TUI does to a **real** Windows console window — opens a conhost, attaches, records buffer and geometry before and after, types real `KEY_EVENT` records, reads the screen back as JSON. For questions the in-process tests cannot answer. |
| `rpc_probe.py` | Drives `epsilon --rpc` from a script and reports what happened, frame by frame. For checking the wire rather than the types that cross it. |
| `desktop-a11y-probe.ps1` | Reads the desktop window's accessibility tree through **Windows UI Automation** — the API NVDA and Narrator are built on — from a **separate process**. Every accessibility test in the repository reads egui's own `accesskit::TreeUpdate` from inside the process that produced it, so this is the only check that the tree reaches the operating system at all. **Green since M312: every node is named**, including the `ScrollBar` this probe used to exit 1 on — and it is named only when egui really drew one, which is the part that took the work. **Still not a battery gate**, now for the honest reason that it needs a desktop session and a window on screen. Its first version matched winit's internal *Thread Event Target* window, reported zero descendants, and exited 0 — a check that cannot see a thing reading as a check that found nothing — so it now requires a real `Window` with children and exits 2 when it finds none. |
| `desktop-reader-walk.ps1` | Walks the same tree the way a **screen reader** walks it and prints the sentence each node would produce — role, name, value, and the states that change what is said — then the findings that are defects rather than style: an unnamed node, **two nodes sharing both a role and a name** (the check that found the two identically-named `File · README.md` rows at M312), a focusable node with no name, and a name long enough to be re-read on every repaint. Exits non-zero on any of them, so it is a check and not only a report. **It runs no reader and captures no audio** — what a reader *sounds like* is that reader's business, and the script says so in its own output rather than implying otherwise. |
| `desktop-frames` (an `#[ignore]`d test, not a script) | Renders four desktop states to PNGs under `EPSILON_FRAME_DIR` — a permission ask, a narrow window, the sessions list, the first run — so **a person can look at them**. The counterpart of `epsilon-tui`'s `frame_dump`, reading the same variable. Run it with `WGPU_BACKEND=gl` on a machine where Vulkan takes the process down. **Looking at the first frame is what found the header-overlap defect fixed at M312**, and it is the only instrument in the repository that can see a thing drawn in the wrong *place* — every other one records which strings appeared (M312). |
| `regen_catalog.py` | Regenerates `crates/epsilon-llm/src/catalog_data.rs` from [models.dev](https://models.dev). The generated file is ~8,000 lines and is not hand-edited. |
| `fix-links.py` | Repairs the relative links left by the 2026-09-28 audit renumbering. One-off; kept because the shape of the breakage is worth remembering. |

## Conventions here

Each check opens with a **doc comment explaining why it exists**, usually with the incident that
made it a gate. That is deliberate: a check whose reason has been forgotten gets deleted when it
becomes inconvenient, and a check whose reason is written down does not.