//! Operator questions: the model's clarifying-question round-trip.
//!
//! Adapted from opencode's `QuestionV2` shape
//! (`references/opencode-1.18.34/packages/schema/src/question.ts` for the
//! `Option`/`Prompt`/`Answer` vocabulary, `packages/core/src/question.ts` for
//! the `ask`/`reply`/`reject` service split): one call carries a list of
//! prompts, each prompt offers labelled options and/or a free-text answer, and
//! answers come back as label arrays in prompt order.
//!
//! This crate holds only the vocabulary and the service seam. Parking,
//! blocking, and the no-operator fallback live with the implementation (the
//! composition crate provisions the shared hub; the `question` tool only
//! speaks this trait), so this crate stays dependency-light and free of any
//! runtime or frontend edge.

use async_trait::async_trait;

/// One labelled choice inside a prompt.
///
/// Ported from opencode's `Question.Option`: a 1-5-word display label plus
/// the sentence that tells the operator what choosing it means.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct QuestionOption {
    /// Display text (1-5 words, concise). Answers are returned as labels.
    pub label: String,
    /// Explanation of the choice, shown under the label.
    pub description: String,
}

/// One question the model asks the operator.
///
/// Ported from opencode's `Question.Info` (the service-level prompt, which is
/// `Prompt` plus the free-text switch): the tool input keeps the same fields
/// so the model-facing schema and the service shape cannot disagree.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct QuestionPrompt {
    /// The complete question, as the operator reads it.
    pub question: String,
    /// Very short label (max 30 chars) naming the question in tab rows.
    pub header: String,
    /// Available choices. May be empty only when `custom` is true.
    pub options: Vec<QuestionOption>,
    /// Allow selecting more than one choice. Default false.
    pub multiple: bool,
    /// Allow typing a free-text answer (default true). A prompt with no
    /// options and `custom: false` is unanswerable and refused at parse time.
    pub custom: bool,
}

/// One prompt's answer: the selected labels (or the typed text) in order.
///
/// Mirrors opencode's `Question.Answer` (`string[]` per question); the whole
/// call answers with one of these per prompt, in prompt order.
pub type QuestionAnswer = Vec<String>;

/// How one `ask` round-trip can end without answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestionError {
    /// No operator is attached (headless/print/RPC/serve): fail closed and
    /// tell the model to proceed with its best judgment. A hung turn is worse
    /// than a refused one.
    NoOperator(String),
    /// The operator dismissed the question (Esc/reject, or an empty
    /// multi-select): never a fabricated answer.
    Dismissed,
    /// The session went away while the question was parked (reload, resume,
    /// session switch): a question asked in a dead session must not resurrect.
    SessionEnded(String),
}

impl std::fmt::Display for QuestionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QuestionError::NoOperator(detail) => write!(f, "no operator attached: {detail}"),
            QuestionError::Dismissed => write!(f, "the operator dismissed this question"),
            QuestionError::SessionEnded(detail) => write!(f, "session ended: {detail}"),
        }
    }
}

impl std::error::Error for QuestionError {}

/// The operator end of the question round-trip.
///
/// Implemented by the composition crate's shared hub (which the TUI drains and
/// answers) and by test stubs. The tool speaks only this trait, so the tool
/// crate needs no edge to any frontend.
#[async_trait]
pub trait QuestionService: Send + Sync {
    /// Park `questions` until the operator answers, then return one answer
    /// per prompt in order. Blocks the calling turn (like `task`); resolves
    /// to [`QuestionError::NoOperator`] immediately when nobody can answer.
    async fn ask(
        &self,
        session_id: String,
        questions: Vec<QuestionPrompt>,
    ) -> Result<Vec<QuestionAnswer>, QuestionError>;
}

/// Render answers the way the model continues from them.
///
/// Mirrors opencode's `toModelOutput`
/// (`packages/core/src/tool/question.ts:34-45`): each prompt formats as
/// `"question"="answer, answer"` with an `Unanswered` fallback, so the model
/// can continue with the operator's answers in mind.
pub fn format_model_output(questions: &[QuestionPrompt], answers: &[QuestionAnswer]) -> String {
    let formatted = questions
        .iter()
        .enumerate()
        .map(|(index, prompt)| {
            let picked = answers
                .get(index)
                .filter(|a| !a.is_empty())
                .map(|a| a.join(", "))
                .unwrap_or_else(|| "Unanswered".to_string());
            format!("\"{}\"=\"{}\"", prompt.question, picked)
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "User has answered your questions: {formatted}. You can now continue with the user's answers in mind."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(question: &str) -> QuestionPrompt {
        QuestionPrompt {
            question: question.into(),
            header: "h".into(),
            options: vec![QuestionOption {
                label: "yes".into(),
                description: "do it".into(),
            }],
            multiple: false,
            custom: true,
        }
    }

    #[test]
    fn model_output_names_each_question_with_its_labels() {
        let out = format_model_output(
            &[prompt("ship it?"), prompt("which side?")],
            &[vec!["yes".into()], vec!["up".into(), "down".into()]],
        );
        assert!(
            out.contains("\"ship it?\"=\"yes\""),
            "single answer formats bare: {out}"
        );
        assert!(
            out.contains("\"which side?\"=\"up, down\""),
            "multi answers join with comma: {out}"
        );
    }

    #[test]
    fn model_output_marks_the_unanswered_rather_than_inventing() {
        let out = format_model_output(&[prompt("ship it?")], &[]);
        assert!(out.contains("\"ship it?\"=\"Unanswered\""), "{out}");
        let out = format_model_output(&[prompt("ship it?")], &[vec![]]);
        assert!(
            out.contains("Unanswered"),
            "an empty selection is not a default: {out}"
        );
    }

    #[test]
    fn errors_say_which_closed_shape_they_are() {
        assert!(
            QuestionError::NoOperator("headless".into())
                .to_string()
                .contains("no operator")
        );
        assert!(QuestionError::Dismissed.to_string().contains("dismissed"));
        assert!(
            QuestionError::SessionEnded("reload".into())
                .to_string()
                .contains("reload")
        );
    }
}
