# D8050 — the remaining build blocker

**Status: OPEN. Everything else is blocked on it.** Do not re-derive this from
scratch; the ruled-out list below is the expensive part.

## Symptom

`cargo build` fails with `exit=101`:

```
error: failed to run custom build command for `aws-lc-sys v0.44.0`
cl : Command line error D8050 : cannot execute
  'C:\Program Files\...\MSVC\14.44.35207\bin\HostX64\x64\c1.dll':
  failed to get command line into debug records
```

**Deterministic**: 7 occurrences, 3/3 runs, 28 in a full battery. All four
compile stages of the battery fail this way, which reads as a catastrophically
broken tree and is not one.

## The paradox that defines it

Running **the same `cl.exe`, with the same arguments, in the same environment,
by hand, exits 0.** Every time. That is why this took as long as it did: the
failure is not reproducible outside the build, so every isolated experiment
"passes" and proves nothing.

## RULED OUT (measured, do not redo)

| Hypothesis | Result |
|---|---|
| Command line too long | real command is **1,547 chars**, limit 32,767 |
| Spaces in output path | clean with *and* without `My Agent Space` |
| `-Z7` | present in both passing and failing runs |
| MSVC env (`INCLUDE`/`LIB`) | now loaded by `gate.ps1`; `cl.exe` fine |
| `TEMP` / `TMP` | correct and writable |
| Parallelism (8 × `cl.exe`) | passes |
| CWD = build `OUT` dir | passes |
| Non-ASCII in any path | every path verified pure ASCII |
| NASM absent | `AWS_LC_SYS_NO_ASM=1` **silences D8050 but does not fix the build** — so not the cause |
| MSBuildTemp denial | **FIXED** in `gate.ps1`; 0 occurrences across 3 runs |

## Alive

- MSVC toolchain dir is **not user-writable** (`touch` → Permission denied).
- **Avira** antivirus installed.
- Official docs have **D8049** ("too long to fit in debug record") but **not
  D8050** — undocumented error.
- vcpkg #25955 has the identical string; closed as "not enough information",
  no maintainer diagnosis.

## Two of my own errors, recorded

1. **`fix1.ps1` reported `EXIT: 0` and I treated it as a fix.** It was a
   PowerShell artifact — the pipeline's `$LASTEXITCODE` was not cargo's. The
   repeat test then showed `exit=101, D8050=7` on all three runs. **Measure the
   exit code where it happens; do not infer it from a filtered pipeline.**
2. **Four hand-rolled `cl.exe` probes gave wrong answers** because PowerShell
   quoting mangled the arguments (one produced a bogus `D8004`, another a bogus
   `C1083`). They "disproved" things that were never really tested. The probes
   that were trustworthy were the ones that let cargo or vcvars do the work.

## Next action

A subagent is testing the strongest remaining hypothesis: **copy the MSVC
toolchain to a writable path and build against the copy.** That is the one
thing that fits "works by hand, fails under cargo" and that I could not test
myself without a 1.8 GB copy.

Secondary, and possibly more valuable: whether `aws-lc-sys` documents a
prebuilt/pregenerated path that avoids the C compiler entirely.

## Machine

Build on `C:` only, in the repo's `target/`. `CARGO_BUILD_JOBS=1`. One build
owner at a time. `pwsh -File scripts/gate.ps1 <label>`. Fresh label per attempt.
