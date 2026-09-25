# M133 — `serve` mode stops discarding a run it refused

The last piece of the F07 surface, and the one place in the product where a durable-write failure was
still silent.

## What

`serve_mode`'s boot task did `let _ = agent.run(message).await;` with **nothing subscribed to the agent's
notices**. When a durable write failed and the turn was refused, both the run's error and core's own
`Notice` vanished: the process printed only its serving banner. The TUI reports this (notice plus
"run failed") and headless `-p` reports it (stderr, exit 1); the server path was the one that swallowed
it.

An operator now sees, on the server's stderr where previously there was nothing:

```
serve: admitted run failed: internal error: the session file could not record this turn (append: io
error: disk full). The turn is in this session's memory but not on disk: a resume, or the next request,
may be missing it. Check the disk and the session file, then retry the turn.
```

## Key decisions

- **The protocol cannot carry this failure, and saying so is part of the fix.** The `Notice` is a
  **broadcast** event — a live stream, not a replay log — and the boot task starts **before the listener
  binds**, so at that instant no subscriber and no client can exist. Adding a replay buffer or a sticky
  `/state` field would be a server redesign, not a call-site fix. The reasoning lives in the helper's
  doc comment, where the next reader will ask exactly this question.
- **Use the channel this path already reports through.** `epsilon-server`'s `/prompt` route logs
  `serve: admitted run failed: {e}`; the boot task now emits the same shape with the same prefix rather
  than inventing a second vocabulary for the same event.
- **The message names the consequence, not the syscall.** The notice text rides inside
  `RunError::Internal`'s `Display`, so the line says the turn is in memory but not on disk and what to do
  about it — rather than "append failed".

## Gotchas

- **The call site had drifted** from the line the brief cited (`1982` → `2107`), which is normal in a
  file this size and worth expecting when a handoff quotes a line number.
- **Store failures cannot be driven through `serve_mode`.** It builds its own agent and its own store,
  and `epsilon-session`'s own note records that an already-open handle still accepts writes through a
  read-only file, a revoked directory permission and a non-writable attribute — so a genuine fault has to
  be injected. That is why the behavioural test injects one and the **call site is held by a source
  guard** instead.
- **Neither test is sufficient alone, and the control proves it.** With the call site reverted, the
  **source guard failed and the behavioural test passed** — a behavioural test cannot see a discarded
  result, because two paths that agree cannot be told apart by running them. This is the second time
  this project has learned that (F2 was the first), and it is why the guard exists rather than being
  dismissed as a grep.

## Verification

Battery `m133a`: **all five gates exit 0, 1,177 passed / 0 failed / 1 ignored across 61 binaries**.
`epsilon-cli` 39 (lib) + 15 (main, both new tests) + 7 + 3 + 2 + 5, 0 failed.

Tests: `runtime_tests::a_durable_write_failure_on_the_serve_path_reaches_the_server_log` (a real store,
a real injected fault, a real run; asserts the prefix, the store's own reason, the consequence, exactly
one `Notice` on a pre-subscribed receiver, the latched second run's refusal, and the divergence —
history 2, store view 1, provider never entered) and
`runtime_tests::the_serve_boot_task_reports_a_failed_run_instead_of_discarding_it` (the source guard).

The control was restored from a backup kept **outside the repo** and proven **by content** — sha
identical to pre-arm, zero CRLF pairs, one deletion in the diff — not by the copy reporting success.

## What nothing exercises

- **No client-visible frame.** A client is told nothing *during its own prompt*: `POST /prompt` returns
  202 immediately and the failure surfaces only as the next request's refusal. Making it visible is a
  protocol change and is not attempted here — recorded as a limit rather than implied by the log line.
- **The `serve` boot path has no end-to-end test through a real socket**; the reporting layer is tested
  directly and the call site by the guard, so the wiring between them is verified by construction rather
  than by a run.
- `--no-default-features` was not built for this slice (nothing here is feature-gated).
