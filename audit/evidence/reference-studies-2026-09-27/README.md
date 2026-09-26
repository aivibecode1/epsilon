# Reference studies — 2026-09-27

Four read-only subagent studies, one per reference tree, dispatched together
(`delegation deleg_9430e4c1`). Each read the reference source directly and was told to
trust no existing Epsilon analysis.

| file | reference | size |
|---|---|---|
| `dsh-transcript.log` | `references/deepseek-harness-dsh-v0.1.6-alpha.2` | see above |
| `hermes-transcript.log` | `references/hermes-agent-2026.9.14` | see above |
| `opencode.txt` | `references/opencode-1.18.31` | 27 KB |
| `pi.txt` | `references/pi-0.85.1` | 40 KB |

**Provenance note, recorded because getting it wrong would have been worse than not
recording it.** The opencode and pi reports were long enough to be written to disk by the
delegation layer, so `*.txt` are verbatim. The DSH and Hermes reports came back inline and
were never written to a file; the first attempt to archive them globbed a **stale
`subagent-summary-0-*` from a previous session** and copied it as if it were today's. That file
was deleted rather than kept, because an audit that cites evidence from the wrong date is worse
than one that admits the gap. Their complete tool/assistant traces are archived here instead.

**These are subagent self-reports, not verified facts.** Every claim that reached
`audit/12-fresh-reference-audit.md` as a finding about *Epsilon* was re-checked against
Epsilon's own source first. Claims about the *references* are cited to the reference paths the
subagent read and are not independently re-verified.
