# M83 — Reproducible baseline integration

## Scope

Restore source dependency closure after earlier commits included whole modified
files without all their companion modules. This is one forward integration of
existing code, not reconstructed M33–M79 history and not a release qualification.
The owner explicitly authorized this integration and waived further approval for
its scope. No history rewrite or push is authorized.

## Evidence and selection

A HEAD-only export of `350c9ec` failed `cargo check --locked --workspace` with
31 compiler error headers, including seven missing unconditional core modules.
The dirty working tree passed the same check. The original files were preserved
in a local ZIP with a SHA-256 inventory before selection.

Three static reviews defined 96 unique source, manifest, asset and test paths.
The candidate was exported independently from HEAD and overlaid only with those
paths. Contracts module publication and core registry changes were selected by
hunk; unrelated stream folding, registry sorting, core prompt-asset changes,
RPC/HTTP attachments and broad TUI changes were excluded. The root manifest omits
the TUI-only unicode-segmentation addition. Cargo resolved the candidate lockfile;
the working-tree lockfile is not assumed correct for this narrower selection.

The first full battery passed formatting, strict Clippy, demo and real check but
failed three picker tests: the selected catalog had retired their hard-coded
model names. A 97th path, `epsilon-tui/src/picker.rs`, carries only existing
catalog-fixture updates. All production picker code remains HEAD. Assertions for
label matching, exact matching and live/catalog deduplication remain intact.
A broad replacement briefly changed the independent recents assertion; a focused
run caught it and the assertion was restored. No tests were deleted or relaxed.

## Verification status

Final isolated battery: **GATE PASS**, all five gates exit 0. Workspace tests:
**740 passed, 0 failed**, across 54 summaries. Explicit locked real and offline-only
CLI checks both exit 0. The smaller count than the 794-test working tree reflects
excluded owner changes/tests, not deleted candidate tests. Windows was exercised;
Linux and MSRV qualification remain pending.

## Residuals and review disposition

The reviews approve dependency selection conditionally, not security or release
quality. F03–F19 and other recorded audit debt remain open as applicable.
Additional existing-code findings are preserved in the review JSONs: Unicode
byte-offset handling in delimiter neutralization; credential temporary-file and
trust-store preservation concerns; incomplete native Bedrock/Vertex flows;
Responses replay ID mismatch; command-helper bounds; retry edge handling;
doctor diagnostics; callback deadline coverage; and weak signing-vector evidence.
These are accepted as explicit baseline debt for this integration, not silently
fixed, dismissed, or certified safe. The subsequent hardening plan must triage
them alongside existing ledger items. No exploit reproductions or paid calls
were needed for this integration.

## Artifacts

All paths below are relative to `audit/evidence/`:

- `head-dependency-review.md`: missing dependency roots and historical attribution.
- `integration-{core,tools,frontends}-review.json`: reviewed scope and residuals.
- `integration-preserved-manifest.json`: original selected-file backup inventory.
- `integration-final-selection.json`: exact candidate selection and hashes.
- `integration-head-check.log`: isolated HEAD failure.
- `integration-working-check.log`: dirty-tree diagnostic success.
- `gate-integration-candidate.log`: initial candidate battery failure.
- `integration-candidate-test.log`: independent failure confirmation.
- `gate-integration-candidate-final.log`: final battery (passed).
- `integration-locked-{real,offline}.log`: explicit feature checks (passed).

The candidate must match the index after Git's line-ending normalization before
commit. Unselected owner edits remain in place, including partial-file residuals.
