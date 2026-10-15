# M277 — the production-readiness session: five audits, twenty-five fixes, one green battery

**Scope:** the owner's standing goal (bring Epsilon past Pi and DSH combined in
capabilities, usability, reliability and code quality), executed as a campaign: five
read-only audits in parallel, one ranked backlog, five serialized implementation waves,
and the closing evidence — full battery, reinstalled binary, real console probe.

---

## What

The session landed **thirteen commits** (`52b246d`..`0a8b006`). The headline is the last
one's neighbours: `gate-production-20261015-c` **GATE PASS — 15/15 legs, 2,144 passed /
0 failed / 15 ignored across 73 test binaries**, and the shipped binary reinstalled from
the proved bytes and driven in a real console (no window shrink, the frame renders, typed
keys land in the composer — the owner's original freeze report, closed with a conhost
probe rather than a ConPTY test).

| Wave | What landed |
|---|---|
| 0 — settle | The 22-slice uncommitted working stack (each already battery-verified) committed as one record naming every slice's log |
| 1 — structure | `epsilon-core/src/lib.rs` split 4,709 → 417 lines across seven child modules, re-exports keeping every path; the architecture guide re-derived from the tree |
| 2 — honesty | The parity matrix withdrew its dead desktop halves and re-tallied; the ledger reconciled; eight doc-vs-code contradictions fixed; the `WINDOW` table's zero readers filed as proposal 16 |
| 3 — TUI polish | **Twenty frame-visible defects**, each with a named test and a run negative control |
| 4 — core | retry's `quota exceeded` double meaning (ADR-041), the session-store short read, the discarded trust-hash write, CSI residue |
| 5 — capability | **The permission dialog shows the diff it is approving** — a defaulted `Tool::preview` seam merged by the core so every mode inherits it |

## The three lessons the session paid for

**1. An audit's anchors drift — twice in one campaign.** Two of the thirteen re-verified
findings pointed at lines that had moved, and two of the audit's own landed inside test
modules. The fix is the discipline, not the pity: re-derive every anchor at edit time,
and record the corrected premise (the `read_all` "panic" was unreachable; the real defect
underneath it was the anonymous EOF).

**2. A dead child is recoverable.** Two implementation waves died at their last
verification step on provider timeouts. Both had finished their code and most of their
proof; the integrator finished each — re-ran the gates, ran the missing controls, fixed a
clippy method-chain and one test's wrong width assumption — and committed only what had
been re-proved. The rule generalises: inspect the artifacts before re-dispatching.

**3. The disk rule is not superstition.** The second battery attempt failed on exactly the
two `epsilon-tools` wall-clock tests the repo's own notes call load-sensitive — both pass
in isolation. Ten hours of builds had left 30 GB free; clearing the cache (13 GB → 6 MB)
made the same suite pass inside the battery. The failed attempt's log is kept beside the
green one, because a campaign that preserves only its successes is not evidence.

## Queued, with its trigger

The dead-code and dependency-edge cleanup, the test-only `pub` narrowing, OSC-sequence
stripping, the retry countdown's live tick, the remaining reference capabilities
(`changes` panel, question tool, plan mode), the 486-citation sweep, and the catalog's
periodic-refresh automation. Each is recorded in `docs/STATUS.md` with its shape rather
than left half-built — the repository's standing rule, and the reason this session could
stop at a green battery instead of a milestone nobody could check.
