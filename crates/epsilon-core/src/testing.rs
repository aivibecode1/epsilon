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
