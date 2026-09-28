# The installed binary refuses pi's own frame — and says so in pi's own words

**Binary:** `C:\Users\Hp\.cargo\bin\epsilon.exe`, 21,466,112 bytes, mtime **2026-09-28 13:58**,
which maps to commit `60313c2` (M193 closeout). It predates **M194 (16:59)** and **M195
(17:19)** entirely.

**Method:** a real frame into the installed release's stdin, JSON out of its stdout.
Probe: `probe_installed_setmodel.py` (scratch). Not an in-process call — an in-process call
is what hid D29 for two milestones.

## The result

```
ping      -> {"ok":{"pong":true},"success":true,"type":"response"}
modelId   -> {"error":"set_model needs modelId, or a model id alone","success":false}
model_id  -> {"ok":{"model":{"id":"faux-1",...},"model_id":"faux-1"},"success":true}
```

Read those last two lines together:

- `modelId` — **the spelling pi documents** (`rpc-types.ts:33`) and that Epsilon's own
  product contract describes — is **refused**.
- `model_id` — Epsilon's internal Rust field name — is **accepted**.
- And the refusal message says *"set_model needs modelId"* **while refusing the frame that
  sends `modelId`**. The error text was written for pi's spelling; the parser only ever
  accepted the other one. Nobody could have read that message and concluded the client was
  wrong.

This is defect **D32** in the running binary: `#[derive(Deserialize)]` under
`rename_all = "snake_case"` on an enum variant field, with no `alias`, so the wire name was
whatever the Rust field happened to be called. The 1,458-result suite passed anyway, because
every test built its frame out of the Rust identifier — the same shape that produced D26,
D29 and D31 before it.

## Why this matters beyond the bug

**This is what the owner is running.** The binary on the PATH has none of M194 and none of
M195: not D31, not D32, not G1 (`app.clear`), not G2 (`/fork`), not G3, not G9, not K1
(honest cache rate), not C2 (the 1,633-token context correction), not the `@`-cache. Any
TUI behaviour reported against it is a report about M193.

**A stale binary is invisible.** `epsilon --version` prints `epsilon 0.1.0` — a static
string that identifies no commit. There is no way for a user to tell which build they have,
so "I ran it and it still does X" cannot be resolved without a probe like this one. Worth
fixing: a build that cannot say what it is.

## The confirming step

Re-run this probe against the binary produced by the install now in flight. The expected
difference is precise and falsifiable:

| Frame | M193 (installed now) | After M194 |
|---|---|---|
| `modelId` | refused | **accepted** |
| `model_id` | accepted | accepted (alias) |

If `modelId` still fails after the install, the fix did not reach the shipped bytes, and
that is the finding — not a green test suite.
