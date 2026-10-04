//! Verification evidence (port plan item 5; Hermes `agent/verification_evidence.py:99`,
//! `verification_stop.py:158`).
//!
//! `crates/epsilon-core/assets/agent-contract.md` tells the model to run the project's checks
//! after editing. Nothing recorded whether it did, so a run could edit four files, claim success
//! and stop — and the claim was unverifiable from the outside. This module keeps the books: which
//! files were changed, which checks ran afterwards, and whether a particular edit has anything
//! standing behind it.
//!
//! It is deliberately *evidence*, not enforcement: the harness does not run the checks itself and
//! does not refuse to stop. It records what happened, and at the stop point it says once — twice
//! at most — that the edits have nothing behind them. The model decides.
//!
//! Scope note: the log belongs to the agent that owns it. A `task` child's edits are the child's
//! record, not the parent's; the parent is nudged only about its own changes.

use serde_json::Value;

/// Program names whose command line can constitute a check. The first word of a command segment
/// has to be one of these before anything else is considered — `grep -r test .` mentions a check
/// marker and is not a check.
const RUNNERS: &[&str] = &[
    "cargo", "npm", "pnpm", "yarn", "bun", "deno", "node", "go", "pytest", "python", "python3",
    "make", "just", "dotnet", "gradle", "mvn", "npx", "cmake", "meson", "rake", "mix", "swift",
    "zig",
];

/// Programs that *are* the check: running one needs no sub-command to count. A linter is a
/// checker by definition, and `make` / `just` / `./gradlew` run the project's declared pipeline
/// whatever it is.
const SELF_CHECKS: &[&str] = &[
    "tsc",
    "ruff",
    "eslint",
    "mypy",
    "biome",
    "pyright",
    "swiftlint",
    "phpstan",
    "hadolint",
    "vale",
    "pytest",
    "shellcheck",
    "golangci-lint",
    "make",
    "just",
    "rake",
    "./gradlew",
    "./mvnw",
    "cmake",
    "meson",
];

/// Sub-commands / flags that make a runner's invocation a check.
const CHECK_MARKERS: &[&str] = &[
    "test",
    "tests",
    "check",
    "clippy",
    "lint",
    "typecheck",
    "type-check",
    "build",
    "verify",
    "vet",
    "analyze",
    "audit",
    "pytest",
    "rspec",
];

/// Whether a shell command looks like running the project's checks.
///
/// Best-effort by design: the precise version detects the project's *declared* commands (Hermes
/// `agent/verify/recipes.py`). A false positive here only means one missing nudge, which is why
/// the predicate errs toward "yes, that was a check" once a known runner is involved; a false
/// negative would nag a model that did verify.
pub fn is_check_command(command: &str) -> bool {
    for segment in command.split(['&', ';', '|', '\n']) {
        let words: Vec<String> = segment
            .split_whitespace()
            .filter(|w| !w.contains('=')) // drop `VAR=value` prefixes
            .map(|w| {
                w.trim_matches(|c: char| c == '"' || c == '\'')
                    .to_lowercase()
            })
            .collect();
        let Some(first) = words.first() else { continue };
        if SELF_CHECKS.contains(&first.as_str()) {
            return true;
        }
        if RUNNERS.contains(&first.as_str())
            && words[1..].iter().any(|w| {
                // Flags count: `cargo test`, `node --test`, `cargo clippy --workspace`.
                let bare = w.trim_start_matches('-');
                CHECK_MARKERS
                    .iter()
                    .any(|m| bare == *m || bare.starts_with(&format!("{m}:")))
            })
        {
            return true;
        }
    }
    false
}

/// Whether a shell result describes a command that **passed**, as distinct from one that was
/// merely *delivered*.
///
/// This is the F05 correction, and the distinction is not cosmetic. `ShellTool` reports a
/// command exiting nonzero inside `Ok(ToolOutput)`: the tool ran fine, the command failed.
/// Deriving verification from `!result.is_error` therefore records "cargo test" with six
/// failing tests as a *passing* check, and the harness then has nothing to say to a model that
/// shipped broken code. The exit code is already published in `details`, so read it.
///
/// A missing or non-numeric `exit_code` — a process killed by a signal, or a tool that reports
/// none — is **not** a pass. An unknown outcome stays unknown, because a redundant nudge costs
/// one message and a false all-clear costs a user a broken afternoon. This deliberately does
/// not touch `is_error`: an ordinary nonzero exit remains a successful *tool call* carrying a
/// failed *command*, and only the verification record distinguishes them.
pub fn check_passed(details: &Value) -> bool {
    matches!(details.get("exit_code").and_then(Value::as_i64), Some(0))
}

/// One check the model ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckRun {
    /// The command as issued.
    pub command: String,
    /// Whether it **exited zero** — not merely whether the tool call was delivered.
    pub ok: bool,
    /// Which turn it ran in (informational; ordering uses `seq`).
    pub turn: usize,
    /// Position in the observation stream, so "after this edit" is exact even within one turn.
    pub seq: usize,
}

/// Files the model changed and checks it ran, in order.
#[derive(Debug, Default)]
pub struct VerificationLog {
    /// `(path, turn of the write, position of the write)` in first-touch order.
    edits: Vec<(String, usize, usize)>,
    checks: Vec<CheckRun>,
    /// How many calls have been observed. Monotonic, so it orders edits against checks even
    /// when both happen in the same turn — which is the case the turn number alone got wrong.
    seen: usize,
}

impl VerificationLog {
    /// A fresh log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one settled tool call. `ok` is the call's own success, not the check's.
    ///
    /// For a **check** the caller must pass whether the command *exited zero* — see
    /// [`check_passed`]. For an **edit** the call's own success is the right test, since a
    /// refused write changed nothing and must not be recorded as a change.
    pub fn observe(&mut self, tool: &str, args: &Value, ok: bool, turn: usize) {
        // One position per call, taken before any early return, so a rejected call still
        // consumes its slot and the ordering never depends on whether something was recorded.
        let seq = self.seen;
        self.seen += 1;
        match tool {
            "write" | "edit" => {
                if !ok {
                    return; // a refused or failed mutation changed nothing
                }
                let Some(path) = args.get("path").and_then(Value::as_str) else {
                    return;
                };
                match self.edits.iter_mut().find(|(p, _, _)| p == path) {
                    Some(entry) => {
                        entry.1 = turn;
                        entry.2 = seq;
                    }
                    None => self.edits.push((path.to_string(), turn, seq)),
                }
            }
            "bash" => {
                let Some(command) = args.get("command").and_then(Value::as_str) else {
                    return;
                };
                if is_check_command(command) {
                    self.checks.push(CheckRun {
                        command: command.to_string(),
                        ok,
                        turn,
                        seq,
                    });
                }
            }
            _ => {}
        }
    }

    /// Whether anything was changed at all.
    pub fn has_edits(&self) -> bool {
        !self.edits.is_empty()
    }

    /// Paths changed with no *successful* check that ran **after** the change.
    ///
    /// A failing check does not clear an edit — that is the whole point of the record: the model
    /// has evidence that something is wrong, and "I ran the tests" is not "the tests pass".
    ///
    /// Ordering is by `seq`, not by `turn`, and that is the F05 correction. Two calls in the same
    /// turn share a turn number, so `turn >=` let a check that ran *before* an edit in the same
    /// batch vouch for the edit it never saw — the model would edit a file, and a check from
    /// earlier in its own batch would "verify" it. `seq >` asks the question that was meant:
    /// did a check run *after* this change?
    pub fn unverified(&self) -> Vec<String> {
        self.edits
            .iter()
            .filter(|(_, _, seq)| !self.checks.iter().any(|c| c.ok && c.seq > *seq))
            .map(|(path, _, _)| path.clone())
            .collect()
    }

    /// The message to hand the model at the stop point, or `None` when there is nothing to say.
    pub fn stop_nudge(&self) -> Option<String> {
        let unverified = self.unverified();
        if unverified.is_empty() {
            return None;
        }
        let listed = unverified
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let more = unverified.len().saturating_sub(3);
        let tail = if more > 0 {
            format!(" (+{more} more)")
        } else {
            String::new()
        };
        let checked = if self.checks.is_empty() {
            "no check has run".to_string()
        } else {
            format!(
                "{} check(s) ran, none succeeding after the change",
                self.checks.len()
            )
        };
        Some(format!(
            "[harness] you changed {listed}{tail}, and {checked}. Run the project's checks \
             (e.g. `cargo test`, `npm test`, `pytest`) and report what they said — or state \
             plainly why no check is needed here."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_runners_that_count_as_checks_and_the_ones_that_do_not() {
        for command in [
            "cargo test",
            "cargo check -p epsilon-core",
            "cargo clippy --workspace",
            "npm test",
            "npm run build",
            "pnpm lint",
            "go test ./...",
            "pytest -q",
            "python -m pytest tests/",
            "make",
            "just check",
            "tsc --noEmit",
            "ruff check .",
            "cd crates/foo && cargo test",
            "RUST_LOG=debug cargo test",
        ] {
            assert!(is_check_command(command), "should be a check: {command}");
        }
        for command in [
            "ls -la",
            "grep -r test .", // mentions a marker, is not a check
            "cat build.log",  // same
            "cargo fmt",      // style, not verification
            "git commit -m 'run test'",
            "echo 'tests pass'",
            "rg --files | head",
        ] {
            assert!(
                !is_check_command(command),
                "should not be a check: {command}"
            );
        }
    }

    #[test]
    fn an_edit_with_no_check_is_unverified() {
        let mut log = VerificationLog::new();
        log.observe(
            "write",
            &json!({"path": "src/a.rs", "content": "x"}),
            true,
            1,
        );
        log.observe(
            "edit",
            &json!({"path": "src/b.rs", "old_string": "a", "new_string": "b"}),
            true,
            2,
        );
        assert_eq!(log.unverified(), vec!["src/a.rs", "src/b.rs"]);
        let nudge = log.stop_nudge().expect("a nudge");
        assert!(nudge.contains("src/a.rs, src/b.rs"), "{nudge}");
        assert!(nudge.contains("no check has run"), "{nudge}");
        assert!(nudge.contains("cargo test"), "it says what to run: {nudge}");
    }

    // ---- F05: delivery is not passage, and order is not the turn number --------------

    #[test]
    fn a_command_that_exited_nonzero_is_delivered_but_did_not_pass() {
        // The exact shape ShellTool produces: the tool worked, the command failed.
        let failed = json!({"exit_code": 101, "duration_ms": 12, "truncated": false});
        assert!(
            !check_passed(&failed),
            "a nonzero exit is not a passing check"
        );
        assert!(check_passed(&json!({"exit_code": 0})), "zero is");
    }

    #[test]
    fn an_unknown_outcome_stays_unknown() {
        // A signalled process reports no exit code. Treating that as a pass would turn a
        // killed test run into a clean bill of health.
        assert!(!check_passed(&json!({"exit_code": null})));
        assert!(!check_passed(&json!({"duration_ms": 9})));
        assert!(!check_passed(&json!({})));
    }

    #[test]
    fn a_failing_exit_leaves_the_edit_unverified() {
        let mut log = VerificationLog::new();
        log.observe("edit", &json!({"path": "src/a.rs"}), true, 1);
        // delivered = true, but the command exited 1 — so it is not evidence.
        log.observe("bash", &json!({"command": "cargo test"}), false, 2);
        assert_eq!(log.unverified(), vec!["src/a.rs"]);
    }

    #[test]
    fn a_check_before_an_edit_in_the_same_turn_does_not_cover_it() {
        // Both calls in turn 1. Under the old `turn >=` rule the check covered the edit,
        // because they share a turn number — the model would edit a file and its own earlier
        // check would vouch for a change that check never saw.
        let mut log = VerificationLog::new();
        log.observe("bash", &json!({"command": "cargo test"}), true, 1);
        log.observe("edit", &json!({"path": "src/a.rs"}), true, 1);
        assert_eq!(
            log.unverified(),
            vec!["src/a.rs"],
            "a check that ran before the edit cannot verify it"
        );
    }

    #[test]
    fn a_check_after_an_edit_in_the_same_turn_does_cover_it() {
        // The positive control for the test above: ordering, not turn membership.
        let mut log = VerificationLog::new();
        log.observe("edit", &json!({"path": "src/a.rs"}), true, 1);
        log.observe("bash", &json!({"command": "cargo test"}), true, 1);
        assert!(
            log.unverified().is_empty(),
            "checked after the edit, same turn"
        );
    }

    #[test]
    fn a_rejected_call_still_consumes_its_position() {
        // A refused write is not an edit, but it happened. If it did not advance the
        // counter, a later check would be credited with a position it never occupied.
        let mut log = VerificationLog::new();
        log.observe("write", &json!({"path": "src/a.rs"}), false, 1); // refused
        log.observe("edit", &json!({"path": "src/a.rs"}), true, 1);
        log.observe("bash", &json!({"command": "cargo test"}), true, 1);
        assert!(log.unverified().is_empty(), "{:?}", log.unverified());
    }

    #[test]
    fn a_successful_check_after_the_edit_clears_it() {
        let mut log = VerificationLog::new();
        log.observe("edit", &json!({"path": "src/a.rs"}), true, 1);
        assert_eq!(log.unverified().len(), 1);
        log.observe("bash", &json!({"command": "cargo test"}), true, 2);
        assert!(log.unverified().is_empty(), "checked after the edit");
        assert!(log.stop_nudge().is_none());
    }

    #[test]
    fn a_failing_check_does_not_count_as_verification() {
        // "I ran the tests" is not "the tests pass" — the record has to keep the distinction.
        let mut log = VerificationLog::new();
        log.observe("edit", &json!({"path": "src/a.rs"}), true, 1);
        log.observe("bash", &json!({"command": "cargo test"}), false, 2);
        assert_eq!(log.unverified(), vec!["src/a.rs"]);
        let nudge = log.stop_nudge().unwrap();
        assert!(
            nudge.contains("none succeeding after the change"),
            "{nudge}"
        );
    }

    #[test]
    fn an_edit_after_a_passing_check_needs_its_own_check() {
        let mut log = VerificationLog::new();
        log.observe("edit", &json!({"path": "src/a.rs"}), true, 1);
        log.observe("bash", &json!({"command": "cargo test"}), true, 2);
        log.observe("edit", &json!({"path": "src/a.rs"}), true, 3);
        assert_eq!(
            log.unverified(),
            vec!["src/a.rs"],
            "the check predates the latest change to this file"
        );
    }

    #[test]
    fn a_failed_or_refused_mutation_is_not_an_edit() {
        let mut log = VerificationLog::new();
        log.observe("write", &json!({"path": "src/a.rs"}), false, 1);
        assert!(!log.has_edits());
        assert!(
            log.stop_nudge().is_none(),
            "nothing changed, nothing to verify"
        );
    }

    #[test]
    fn read_only_work_is_never_nudged() {
        let mut log = VerificationLog::new();
        log.observe("read", &json!({"path": "src/a.rs"}), true, 1);
        log.observe("grep", &json!({"pattern": "fn main"}), true, 1);
        log.observe("bash", &json!({"command": "ls -la"}), true, 2);
        assert!(log.stop_nudge().is_none());
    }

    #[test]
    fn the_nudge_lists_at_most_three_paths_and_counts_the_rest() {
        let mut log = VerificationLog::new();
        for i in 0..5 {
            log.observe("write", &json!({"path": format!("src/f{i}.rs")}), true, 1);
        }
        let nudge = log.stop_nudge().unwrap();
        assert!(nudge.contains("src/f0.rs, src/f1.rs, src/f2.rs"), "{nudge}");
        assert!(nudge.contains("(+2 more)"), "{nudge}");
    }
}
