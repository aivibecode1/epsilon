//! The sandbox mode vocabulary and the escalation ladder (port plan item 15; DSH
//! `packages/sandbox/sandbox/src/escalation.ts:28,51,71,85,157`).
//!
//! Three modes, ordered by how much they permit:
//!
//! | Mode | Means |
//! |---|---|
//! | [`SandboxMode::ReadOnly`] | the command may look, not touch |
//! | [`SandboxMode::WorkspaceWrite`] | the command may change the workspace (the default) |
//! | [`SandboxMode::DangerFullAccess`] | no confinement at all — the trusted-automation tier |
//!
//! The effective mode is **per-call truth**, not a schema enum: a tool schema is
//! registry-global, while what a given call is allowed to do depends on the session's
//! mode and on what the user approves for *that* call. DSH pins the same reasoning at
//! `escalation.ts:24`, and the consequence is that widening a mode is an *execution*
//! check — see [`SandboxMode::is_strictly_wider_than`] and the fail-closed sequence the
//! caller must run (validate → ask → map the outcome) before anything spawns.
//!
//! Narrowing is always allowed and never needs approval: a call that asks for less than
//! the session's mode is asking for a *stronger* guarantee.

use std::fmt;

/// How much a confined command is allowed to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SandboxMode {
    /// Read and execute, change nothing.
    ReadOnly,
    /// Read and write inside the workspace (the default mode).
    WorkspaceWrite,
    /// No confinement: the command runs with the harness's own privileges.
    DangerFullAccess,
}

impl SandboxMode {
    /// Every mode, narrowest first. This is the *closed advertisement vocabulary* the
    /// model may ask for — deliberately not narrowed to the modes wider than some
    /// session default, because a session whose effective mode sits below the top of the
    /// ladder must still be able to name what it wants (DSH `ESCALATION_TARGETS`).
    pub const ALL: [SandboxMode; 3] = [
        SandboxMode::ReadOnly,
        SandboxMode::WorkspaceWrite,
        SandboxMode::DangerFullAccess,
    ];

    /// The wire name (what a call may put in `sandbox_permissions`).
    pub fn as_str(self) -> &'static str {
        match self {
            SandboxMode::ReadOnly => "read-only",
            SandboxMode::WorkspaceWrite => "workspace-write",
            SandboxMode::DangerFullAccess => "danger-full-access",
        }
    }

    /// Parse a wire name. Unknown names are an error, never a silent default: a typo in
    /// `sandbox_permissions` must not quietly become either the narrowest or the widest
    /// mode.
    pub fn parse(name: &str) -> Result<Self, EscalationError> {
        match name.trim() {
            "read-only" => Ok(SandboxMode::ReadOnly),
            "workspace-write" => Ok(SandboxMode::WorkspaceWrite),
            "danger-full-access" => Ok(SandboxMode::DangerFullAccess),
            other => Err(EscalationError::UnknownMode(other.to_string())),
        }
    }

    /// Modes strictly wider than this one, i.e. the only legal escalation targets.
    ///
    /// Deliberately a table and not `mode > current`: the ladder is a policy statement,
    /// so it stays readable (and reviewable) as a list.
    pub fn wider(self) -> &'static [SandboxMode] {
        match self {
            SandboxMode::ReadOnly => &[SandboxMode::WorkspaceWrite, SandboxMode::DangerFullAccess],
            SandboxMode::WorkspaceWrite => &[SandboxMode::DangerFullAccess],
            SandboxMode::DangerFullAccess => &[],
        }
    }

    /// Is `self` a strictly wider mode than `current`? The one predicate the ladder
    /// turns on — equal modes are not an escalation, and neither is narrowing.
    pub fn is_strictly_wider_than(self, current: SandboxMode) -> bool {
        // Read it as "am I in the set of modes that are wider than `current`". Table-only on
        // purpose: a new variant makes the match in `wider()` non-exhaustive and will not
        // compile until someone decides where it sits on the ladder, whereas an ordering
        // comparison would widen silently.
        current.wider().contains(&self)
    }

    /// One line for the model and the transcript.
    pub fn describe(self) -> &'static str {
        match self {
            SandboxMode::ReadOnly => "read-only (the command may look, not touch)",
            SandboxMode::WorkspaceWrite => "workspace-write (the workspace can be changed)",
            SandboxMode::DangerFullAccess => "danger-full-access (no confinement)",
        }
    }
}

impl fmt::Display for SandboxMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why an escalation request was refused before it could reach the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EscalationError {
    /// `sandbox_permissions` and `justification` must travel together.
    ArgsMustTravelTogether,
    /// The justification was empty, or a fragment rather than a sentence.
    JustificationNotASentence,
    /// The requested mode is not in the vocabulary.
    UnknownMode(String),
    /// The request is not a strict widening, so it is not an escalation at all.
    NotAWidening {
        /// The session's effective mode.
        from: SandboxMode,
        /// What the call asked for.
        to: SandboxMode,
    },
}

impl fmt::Display for EscalationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EscalationError::ArgsMustTravelTogether => write!(
                f,
                "`sandbox_permissions` needs a `justification` sentence beside it (and the \
                 reverse): the two arguments describe one decision"
            ),
            EscalationError::JustificationNotASentence => write!(
                f,
                "`justification` must be a sentence saying why the wider mode is needed"
            ),
            EscalationError::UnknownMode(name) => write!(
                f,
                "unknown sandbox mode {name:?}; the vocabulary is: {}",
                SandboxMode::ALL
                    .iter()
                    .map(|m| m.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            EscalationError::NotAWidening { from, to } => write!(
                f,
                "{to} is not wider than the session's mode ({from}); only widening needs approval"
            ),
        }
    }
}

impl std::error::Error for EscalationError {}

/// What the call asked about its confinement: a mode with the justification that must
/// accompany a widening (DSH `validateEscalationArgs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EscalationRequest {
    /// The mode the call wants.
    pub mode: SandboxMode,
    /// Why — non-empty, a sentence.
    pub justification: String,
}

/// Validate the paired arguments a call may carry.
///
/// Returns `Ok(None)` when the call said nothing about its mode (the common case: the
/// session default applies), `Ok(Some(req))` when it asked for a *narrower or equal*
/// mode (allowed outright), and the caller runs the approval sequence for a widening.
/// Any malformed pairing is an error, never a default — a call that says
/// `sandbox_permissions` with no reason must be told, not guessed at.
pub fn validate_escalation_args(
    mode: Option<&str>,
    justification: Option<&str>,
) -> Result<Option<EscalationRequest>, EscalationError> {
    match (mode, justification) {
        (None, None) => Ok(None),
        (Some(_), None) | (None, Some(_)) => Err(EscalationError::ArgsMustTravelTogether),
        (Some(mode), Some(justification)) => {
            let mode = SandboxMode::parse(mode)?;
            let sentence = justification.trim();
            if sentence.len() < 8 || !sentence.contains(char::is_whitespace) {
                return Err(EscalationError::JustificationNotASentence);
            }
            Ok(Some(EscalationRequest {
                mode,
                justification: sentence.to_string(),
            }))
        }
    }
}

/// Decide whether a call needs an approval ask, given the session's effective mode.
pub fn escalation_target(
    requested: &EscalationRequest,
    session: SandboxMode,
) -> Result<Option<SandboxMode>, EscalationError> {
    if requested.mode.is_strictly_wider_than(session) {
        Ok(Some(requested.mode))
    } else {
        Err(EscalationError::NotAWidening {
            from: session,
            to: requested.mode,
        })
    }
}

/// The audit reason an approval ask carries — self-contained, so an approval log read
/// later needs no other context (DSH `escalate sandbox to ${mode}: ${justification}`).
pub fn escalation_reason(target: SandboxMode, justification: &str) -> String {
    format!("escalate sandbox to {target}: {}", justification.trim())
}

/// What a call is told when an escalation was refused. Model-facing text, deliberately
/// shaped as a *statement of fact plus the next step*, because a bare "denied" invites a
/// retry with the same arguments.
pub fn denial_marker(mode: SandboxMode) -> String {
    format!(
        "[sandbox] request for {mode} was not approved. The command did not run. \
         Continue at the current mode, or explain to the user what the wider mode would \
         let you do."
    )
}

/// The one-retry nudge: the call may ask again *with arguments* (DSH
/// `escalationHintMarker`).
pub fn escalation_hint(text: &str) -> String {
    format!(
        "[sandbox] to run {text} with a wider mode, repeat the call with \
         `sandbox_permissions` and a `justification` sentence so the user can approve it."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ladder_strictly_widens_and_never_wraps() {
        // The table's shape is the policy: three rungs, each widening to everything above it.
        assert_eq!(
            SandboxMode::ReadOnly.wider(),
            &[SandboxMode::WorkspaceWrite, SandboxMode::DangerFullAccess]
        );
        assert_eq!(
            SandboxMode::WorkspaceWrite.wider(),
            &[SandboxMode::DangerFullAccess]
        );
        assert!(SandboxMode::DangerFullAccess.wider().is_empty());

        for mode in SandboxMode::ALL {
            assert!(
                !mode.is_strictly_wider_than(mode),
                "{mode} must not be an escalation from itself"
            );
            for wider in mode.wider() {
                assert!(wider.is_strictly_wider_than(mode), "{wider} > {mode}");
                assert!(!mode.is_strictly_wider_than(*wider), "and not the reverse");
            }
        }
    }

    #[test]
    fn the_table_agrees_with_the_declared_order() {
        // `wider()` is the policy people read; the `Ord` derive is what `ALL` sorts by. They
        // must not drift apart, or the ladder would say one thing and the vocabulary another.
        for mode in SandboxMode::ALL {
            let from_order: Vec<SandboxMode> = SandboxMode::ALL
                .iter()
                .copied()
                .filter(|other| *other > mode)
                .collect();
            assert_eq!(
                mode.wider().to_vec(),
                from_order,
                "the ladder and the order disagree at {mode}"
            );
        }
    }

    #[test]
    fn narrowing_is_never_an_escalation() {
        // The safety direction: asking for less than the session runs at must not need approval.
        assert!(!SandboxMode::ReadOnly.is_strictly_wider_than(SandboxMode::WorkspaceWrite));
        assert!(!SandboxMode::WorkspaceWrite.is_strictly_wider_than(SandboxMode::DangerFullAccess));
        let narrowed = EscalationRequest {
            mode: SandboxMode::ReadOnly,
            justification: "just looking".into(),
        };
        assert_eq!(
            escalation_target(&narrowed, SandboxMode::WorkspaceWrite),
            Err(EscalationError::NotAWidening {
                from: SandboxMode::WorkspaceWrite,
                to: SandboxMode::ReadOnly
            })
        );
    }

    #[test]
    fn the_paired_arguments_must_travel_together() {
        assert_eq!(validate_escalation_args(None, None), Ok(None));
        assert_eq!(
            validate_escalation_args(Some("danger-full-access"), None),
            Err(EscalationError::ArgsMustTravelTogether)
        );
        assert_eq!(
            validate_escalation_args(None, Some("because")),
            Err(EscalationError::ArgsMustTravelTogether)
        );
    }

    #[test]
    fn a_justification_must_be_a_sentence_and_a_mode_must_be_known() {
        assert_eq!(
            validate_escalation_args(Some("danger-full-access"), Some("because")),
            Err(EscalationError::JustificationNotASentence)
        );
        assert_eq!(
            validate_escalation_args(Some("danger-full-access"), Some("   ")),
            Err(EscalationError::JustificationNotASentence)
        );
        assert_eq!(
            validate_escalation_args(Some("full-access"), Some("I need to write outside")),
            Err(EscalationError::UnknownMode("full-access".into()))
        );
        let ok = validate_escalation_args(
            Some("danger-full-access"),
            Some("the build writes outside the workspace tree"),
        )
        .expect("a complete request validates");
        assert_eq!(
            ok,
            Some(EscalationRequest {
                mode: SandboxMode::DangerFullAccess,
                justification: "the build writes outside the workspace tree".into(),
            })
        );
    }

    #[test]
    fn an_unknown_mode_is_never_silently_the_default_or_the_widest() {
        // The typo that must not become `DangerFullAccess`.
        assert!(SandboxMode::parse("Danger-Full-Access").is_err());
        assert!(SandboxMode::parse("").is_err());
        assert!(SandboxMode::parse("workspace").is_err());
        for mode in SandboxMode::ALL {
            assert_eq!(SandboxMode::parse(mode.as_str()), Ok(mode));
        }
    }

    #[test]
    fn the_audit_reason_and_the_model_facing_text_say_what_happened() {
        let reason = escalation_reason(SandboxMode::DangerFullAccess, "  the build needs it  ");
        assert_eq!(
            reason,
            "escalate sandbox to danger-full-access: the build needs it"
        );
        let denied = denial_marker(SandboxMode::DangerFullAccess);
        assert!(denied.contains("was not approved"), "{denied}");
        assert!(denied.contains("did not run"), "{denied}");
        let hint = escalation_hint("the build");
        assert!(
            hint.contains("sandbox_permissions") && hint.contains("justification"),
            "{hint}"
        );
    }
}
