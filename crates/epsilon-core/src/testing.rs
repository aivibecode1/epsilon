//! Test fixtures shared by loop integration tests and the demo binary.
//!
//! `#[doc(hidden)]`: not public API, but compiled into the crate so tests and demos use
//! exactly the same tool implementations.

use async_trait::async_trait;
use epsilon_contracts::tool::{
    ExecutionMode, Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation, ToolOutput,
    ToolUpdateSink,
};

/// Echoes its `text` argument back; parallel-safe. The workhorse of scripted scenarios.
pub struct EchoTool {
    descriptor: ToolDescriptor,
}

impl EchoTool {
    /// Build with the default name `echo`.
    pub fn new() -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: "echo".into(),
                label: "Echo".into(),
                description: "Returns its input text unchanged.".into(),
                parameters_schema: serde_json::json!({
                    "type": "object",
                    "properties": {"text": {"type": "string"}},
                    "required": ["text"]
                }),
                execution_mode: ExecutionMode::Parallel,
            },
        }
    }
}

impl Default for EchoTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for EchoTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        _ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        let text = call
            .args
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        Ok(ToolOutput::text(text))
    }
}

/// Always fails with a fixed message — exercises the error-result path.
pub struct FailTool {
    descriptor: ToolDescriptor,
}

/// Wraps any provider with a fixed pre-stream delay — makes timing-sensitive tests
/// deterministic without polling hacks.
pub struct SlowProvider<S> {
    inner: S,
    delay_ms: u64,
}

impl<S> SlowProvider<S> {
    /// Wrap `inner` with a `delay_ms` pause before each stream opens.
    pub fn new(inner: S, delay_ms: u64) -> Self {
        Self { inner, delay_ms }
    }
}

#[async_trait]
impl<S: epsilon_llm::StreamFn + Send + Sync> epsilon_llm::StreamFn for SlowProvider<S> {
    async fn stream(
        &self,
        model: &epsilon_llm::ModelRef,
        ctx: epsilon_llm::LlmContext,
    ) -> epsilon_llm::BoxEventStream {
        tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        self.inner.stream(model, ctx).await
    }
}

impl FailTool {
    /// Build with the default name `fail`.
    pub fn new() -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: "fail".into(),
                label: "Fail".into(),
                description: "Always fails; used to test error results.".into(),
                parameters_schema: serde_json::json!({"type": "object"}),
                execution_mode: ExecutionMode::Parallel,
            },
        }
    }
}

impl Default for FailTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Sleeps for a fixed delay, counts executions, and deliberately ignores the run
/// token: a test that aborts *during* a call needs one call still in flight when the
/// abort lands, and needs to observe whether a later call in the same batch ever
/// started.
///
/// `Sequential` with no declared resource, so a batch containing it is a barrier —
/// its calls run one at a time, in the order the model asked for them (exec.rs
/// `plan_batch`).
pub struct SlowTool {
    descriptor: ToolDescriptor,
    delay_ms: u64,
    runs: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl SlowTool {
    /// Build with the default name `slow`.
    pub fn new(delay_ms: u64) -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: "slow".into(),
                label: "Slow".into(),
                description: "Sleeps, then echoes its input; counts executions.".into(),
                parameters_schema: serde_json::json!({
                    "type": "object",
                    "properties": {"text": {"type": "string"}},
                    "required": ["text"]
                }),
                execution_mode: ExecutionMode::Sequential,
            },
            delay_ms,
            runs: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// How many times this tool actually started executing (counted before the
    /// sleep, so a refused call is never counted).
    pub fn runs(&self) -> std::sync::Arc<std::sync::atomic::AtomicUsize> {
        std::sync::Arc::clone(&self.runs)
    }
}

#[async_trait]
impl Tool for SlowTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        _ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        self.runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        let text = call
            .args
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        Ok(ToolOutput::text(text))
    }
}

#[async_trait]
impl Tool for FailTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        _call: ToolInvocation,
        _ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        Err(ToolFailure::msg("intentional failure"))
    }
}
