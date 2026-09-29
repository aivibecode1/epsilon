# M206 — the goal on the wire

**Status:** landed. GATE PASS `m206` — see [Verification](#verification). Reinstall: **required**
(a client-facing payload changed).

## What

`/goal` was reachable from the keyboard and from the model, and from nowhere else: the two wire
surfaces (`rpc get_state` and `GET /state`) serve one shared `SessionState` payload, and it had no
place to put a goal. A client — a GUI on top of the RPC, a dashboard polling `/state`, a script —
could see the model, the context estimate and the session name, and could not see what the session
is *for*.

`SessionState` gains `goal`, and `null` means none:

```json
{
  "model": "…", "thinkingLevel": "low", "isStreaming": false,
  "estimatedContextTokens": 41000,
  "goal": {
    "id": "goal_1a2b…", "revision": 7, "objective": "make the doc gates pass",
    "status": "active", "roundsStarted": 3, "maxRounds": 256,
    "armed": true, "blocker": null
  }
}
```

## The two decisions inside it

- **A nested object, not flat fields.** `null` has to be distinguishable from "a goal whose
  objective is empty" and from "a client that forgot to ask", and a client rendering a goal strip
  needs to know which it is looking at. `goalStatus: null` cannot say.
- **`armed` is on the wire, and it is the point.** An `active` goal in a fresh process is
  *disarmed* — a goal is durable, the permission to continue it is not — so a client that took
  `status == active` to mean "it will keep going" would render a goal that has stopped.
  `the_wire_payload_carries_the_goal_and_its_activation` asserts exactly that pair: the same
  status, a different activation, and the payload agreeing with the view the TUI renders from.

`SessionGoal` is a flat projection rather than `GoalView` itself: the snapshot carries timestamps a
client has no use for, and keeping the two types apart means a change to the durable snapshot
cannot silently change the wire.

## What changed, exactly

| File | Change |
|---|---|
| `epsilon-contracts/session_state.rs` | `SessionGoal` + the `goal` field, and the module doc's field-set paragraph now counts **two** additive fields rather than one |
| `epsilon-core/lib.rs` | `session_state()` fills it from `self.goal()` — the same constructor both transports already used, so neither could grow a second one (D29's lesson) |
| `epsilon-server/lib.rs` | the whole-key-set assertion gained `"goal"`, plus an assertion that a store-less agent reports `null` |
| `epsilon-core/tests/goal_state.rs` | the round trip, including the status-versus-activation pair |

## Limits — what this does not prove

- **Read-only.** A client can see the goal and cannot change it: there is no `set_goal` op and no
  `POST /goal`. The verbs are a follow-up, and the shape is ready for them — `id` and `revision`
  are on the wire precisely so a mutation can carry the compare-and-set ref the service requires.
- **Nothing consumes it yet.** No in-tree client reads the field; it exists so that one can. A
  field added for a hypothetical consumer is the "faithful port, no production caller" class this
  repository keeps finding — the difference here is that it is *requested* by pi's own protocol
  position (a state payload is what a client polls) and asserted at both surfaces, so it cannot rot
  into a lie. Recorded rather than dressed up.
- **The OpenAPI document is generated from the struct**, so it learned the field for free; nothing
  asserts that the document and the server agree about it, beyond the key-set test.

## Verification

GATE PASS, 2026-09-29 — battery **`m206`**, run against this commit's contents with the
documentation already final. Seven gates, exit 0 each: `check-links` 0 s (454 relative links,
0 broken), `check-lint-tables` 0 s, `fmt --check` 2 s, `clippy --workspace --all-targets` 1 s,
`test --workspace` 641 s (a cold-ish leg: the browser tests and the terminal suite both ran),
`demo` 97 s, `real-check` 16 s.
**1,618 tests across 64 binaries, 0 failed** — summed from the 64 `test result: ok.` lines in the
log. M205 measured 1,617 across the same 64, so this slice adds the one round-trip test.

Evidence: [`audit/evidence/gate-m206.log`](../../audit/evidence/gate-m206.log).

**Reinstall:** required — a client-facing payload changed — and done.
