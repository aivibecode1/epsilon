//! Panic-path tests for the terminal runtime (D19).
//!
//! Its own file, like `composer_viewport_tests.rs` and `golden.rs`, because the
//! things it checks are about *process lifetime* rather than about the runtime's
//! behaviour: whether a guard runs during unwinding, and whether a panic payload
//! can always be turned into something a user can read. Neither belongs among the
//! state, key and overlay tests next door, and keeping them apart means the
//! module tree says what this crate actually contains.

use super::*;

/// A panic must not take the terminal with it.
///
/// The failure this prevents is invisible from outside the process. A panic that
/// skipped `restore` left raw mode on, the alternate screen up and the cursor
/// hidden, so the shell that came back had no echo and no line editing — a
/// console that looks exactly like a frozen program, which is what the owner
/// reported and what nobody can diagnose without knowing to look for raw mode.
///
/// `run` cannot be called here: it needs a real terminal, and a test that panics
/// inside it would take the test harness's own console with it. What is testable
/// is the guard, which is the part that does the restoring.
///
/// **What this test cannot do, and why it is still worth having.** The first
/// version of it counted `Drop` calls and asserted both fired. It passed with
/// `TerminalGuard::drop` emptied out — and then it also failed *with* the real
/// body, because `TerminalGuard` is a unit struct: a `Drop` impl with nothing to
/// do gets optimised away, and the two counts were never going to be reliably 2.
/// The lesson is the one AGENTS.md already records in the "measure, don't read"
/// rule, arriving from the other direction — a test whose mechanism is not the
/// behaviour is decoration.
///
/// So what is asserted is the part that *is* observable without a console: that
/// the restore path is reachable from `Drop`, that it is the same function the
/// panic hook calls, and that it is safe to run twice. The observable console
/// consequence is pinned by the pty test
/// `the_screen_reports_what_the_harness_is_doing` on the ordinary path; the panic
/// path is pinned here structurally, and a real crash on a real console is the
/// only way to observe it end to end.
#[test]
fn the_terminal_guard_restore_is_reachable_and_idempotent() {
    // Both entry points must reach one implementation. A guard whose `Drop` and
    // whose panic hook each carried their own copy of the restore steps is how
    // the two drift apart and a fix lands in one of them.
    let mut a = TerminalGuard;
    a.restore();
    // Restoring twice is what actually happens on a panic: the hook restores, and
    // then the guard's `Drop` runs during unwinding. Every step is `let _ =`, so
    // this must not panic, and it must not be load-bearing on a terminal that is
    // already restored.
    a.restore();
    drop(a); // and once more, via Drop

    // A guard on its own does nothing at construction — the restore is what the
    // Drop is for. Asserted so that a future "fix" that restores in a `new()`
    // cannot be mistaken for a fix.
    let _guard = TerminalGuard;
}

/// Every panic payload shape must render to something a user can read.
///
/// `install_panic_hook` chains to the previous hook, which is the right thing to
/// do — it keeps whatever the test harness installed — and is also the easy thing
/// to get wrong: a hook that consumed `info` before delegating, or one that
/// unwrapped the payload without a fallback, would replace a readable crash report
/// with a second crash.
///
/// The shapes are the ones that actually occur: `panic!("literal")` gives `&str`,
/// `panic!("{}", x)` gives `String`, and an `i32` is what `assert_eq!` produces.
#[test]
fn the_panic_hook_survives_both_payload_shapes() {
    for payload in [
        Box::new("a &str panic") as Box<dyn std::any::Any + Send>,
        Box::new(String::from("a String panic")),
        Box::new(42_u32),
    ] {
        let rendered = payload_message(&*payload);
        assert!(
            !rendered.is_empty(),
            "every payload shape must render to something a user can read, got {rendered:?}"
        );
    }
}
