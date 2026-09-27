# M165 — two self-contained seams, and a coverage hole a control found

## What this did

Two blocks moved out of `app.rs` into files of their own:

| new file | what | lines |
| --- | --- | --- |
| `app/editor.rs` | the external-editor bridge: `$EDITOR` resolution and the suspend/spawn/read-back round trip | 118 |
| `app/login.rs` | the OAuth login adapter: `OauthUiEvent`, `TuiLoginUi`, and the `LoginUi` impl | 69 |

`app.rs` is now **1,892** lines, down from 4,526 at the start of this work in M161. All
seven items are byte-identical to the parent commit apart from the `pub(super)` the move
required, verified per item.

## Why these two, and why they were safe

Both are *self-contained* — the easiest and least dangerous kind of seam. Neither has a
method body reaching into `App`; they are free functions, types, and one foreign-trait impl
that touch nothing else in the module. Both carried a section banner or a doc comment that
already declared them a unit.

`app/editor.rs` also earns its file on the evidence: two of its five functions are already
shaped for testing, and it is visible now. `editor_command_from` takes the three candidate
settings as arguments rather than reading the environment, so the precedence order is a pure
function; `edit_with` takes the spawner as a `&dyn Fn`, so the whole round trip runs with no
real editor, no TTY and no suspend. That is the pattern the rest of the codebase should
copy, and it was invisible while these sat between unrelated items in a 2,000-line file.

## The `--features real` build is not optional here

`app/login.rs` is `#[cfg(feature = "real")]`, because a faux provider has nothing to log in
to. A plain `cargo build` compiles an **empty** file. The first `--features real` build
failed with:

```
error[E0451]: fields `notices`, `events`, `code_rx` and `provider` of struct
`login::TuiLoginUi` are private
```

which the default build could never have found: `App::start_oauth_login` constructs the
struct, and after the move the parent can no longer reach private fields of a child's type.
This is the M13 lesson in a new place — the flag is not ceremony, and the code it gates is
exactly the code nobody looks at.

## The control found a coverage hole, and the milestone closes it

Before trusting the move, the obvious question is whether anything exercises the code that
moved. `OauthUiEvent` has tests — the overlay opening on `NeedCode` and closing on `Done` are
both covered. So the control gutted `TuiLoginUi::notify`:

```rust
fn notify(&mut self, message: &str) {
    let _ = message;
}
```

**364 passed, 0 failed.** The entire `LoginUi` adapter — the only way the OAuth worker thread
can say anything at all — had **no coverage whatsoever**, and because it is `real`-gated, an
ordinary `cargo test` never even builds it. A user whose device login printed nothing would
have got silence from the harness and from the suite in equal measure.

So the milestone adds `the_login_adapter_carries_notices_and_the_device_code_to_the_
transcript`, which constructs the adapter over real channels and asserts the whole of its
observable surface: a notice arrives verbatim, and a device-code line arrives carrying the
verification URI, the user code and the expiry.

Re-running the same control against the new test: **364 passed, 1 failed**, naming it. The
hole is closed and demonstrably so.

This is the discipline the project has been arguing for all along — a field that every card
advertised but no test asserted, a code path no test had ever drawn — found this time by
deliberately breaking something instead of reading the code carefully and feeling pleased.

## Verification

| gate | result |
| --- | --- |
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --workspace --all-targets --features real -- -D warnings` | exit 0 |
| `cargo test --workspace --features real` | **62 binaries, 1,289 passed, 0 failed** |
| `cargo run -q -p epsilon-cli -- --demo` | exit 0, 6 messages, `tokens in/out: 300/150` |

`epsilon-tui` is **365** (was 364): one test added, none removed. The count rose because a
real gap was closed, which is the only reason a count should ever move.

## Limits — what this milestone does *not* prove

- **`impl App #1` in `app.rs` is still 929 lines** and is the largest item in the crate. It
  mixes construction, composer editing, slash/`@` completion, submission, the OAuth bridge,
  model application and live-cache refresh.
- **`TuiLoginUi::paste_code` and the `LoginUi` methods that consume `code_rx` are still only
  covered indirectly.** The new test exercises `notify` and `device_code`; the code path where
  the overlay's pasted code is handed to the waiting thread is exercised through the App's
  own tests, not through the adapter.
- **No real-console run**, because no frame or keypress changed.
- **The other `real`-gated TUI code was not audited this way.** If one block can be invisible
  to the default build and untested under `real`, the others deserve the same control. That
  is a real piece of unfinished work, listed rather than implied.
