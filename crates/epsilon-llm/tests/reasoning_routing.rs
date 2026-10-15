//! Wave D (criterion 5): reasoning request shaping reads the catalog
//! capability field (`ModelInfo::reasoning_shape`), never model-id substrings.
//!
//! Two halves: a source-scanning test pinning the M3 invariant at the four
//! provider files the integrator flagged (it fails if a NEW id-substring
//! behavior branch appears), and per-wire request-body pins proving each
//! family routes to its documented body shape.

#![cfg(feature = "real")]

use epsilon_contracts::ThinkingLevel;
use epsilon_llm::providers::ProviderWire;
use epsilon_llm::providers::bedrock::BedrockConverseWire;
use epsilon_llm::providers::github_copilot::{COPILOT_DEFAULT_BASE_URL, CopilotWire};
use epsilon_llm::providers::mistral::MistralConversationsWire;
use epsilon_llm::providers::openai_compat::OpenAiCompatWire;

fn ctx_at(level: ThinkingLevel) -> epsilon_llm::LlmContext {
    epsilon_llm::LlmContext {
        system_prompt: "sys".to_string(),
        messages: vec![epsilon_contracts::message::Message::User(
            epsilon_contracts::message::UserMessage {
                id: epsilon_contracts::new_id("msg"),
                timestamp_ms: 0,
                content: vec![epsilon_contracts::content::UserContent::Text {
                    text: "hi".to_string(),
                }],
            },
        )],
        tools: Vec::new(),
        thinking_level: level,
    }
}

fn compat_wire() -> OpenAiCompatWire {
    OpenAiCompatWire {
        base_url: "https://openrouter.ai/api/v1".to_string(),
        env_key: Some("OPENROUTER_API_KEY".to_string()),
    }
}

fn mistral_wire() -> MistralConversationsWire {
    MistralConversationsWire {
        base_url: "https://api.mistral.ai".to_string(),
        env_key: Some("MISTRAL_API_KEY".to_string()),
    }
}

fn copilot_compat_wire() -> CopilotWire {
    let inner = Box::new(OpenAiCompatWire {
        base_url: COPILOT_DEFAULT_BASE_URL.to_string(),
        env_key: Some("COPILOT_GITHUB_TOKEN".to_string()),
    });
    CopilotWire::new(
        COPILOT_DEFAULT_BASE_URL,
        Some("COPILOT_GITHUB_TOKEN"),
        None,
        false,
        inner,
    )
}

/// M3 binding property, pinned at the four files the integrator flagged:
/// reasoning-request shaping reads the catalog capability field, never
/// model-id substrings. Source-scanning rather than a behavioral assertion
/// (like `the_home_directory_is_computed_in_exactly_one_place` in
/// epsilon-core): the risk is a NEW branch in any of these files, and only
/// reading the files sees all of them.
///
/// Allowlist: Bedrock's `eu.` region default and ARN region parse read the
/// id's QUALIFICATION (region routing, deliberately outside this field's
/// scope — see the ADR), not its family. Anything else probing a model id
/// with a substring predicate or an id equality is a behavior branch and
/// fails this test.
#[test]
fn reasoning_routing_reads_no_model_id_substrings() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let files = [
        "src/providers/openai_compat.rs",
        "src/providers/mistral.rs",
        "src/providers/bedrock.rs",
        "src/providers/github_copilot.rs",
    ];
    let allowed: &[(&str, &str)] = &[
        ("src/providers/bedrock.rs", "model_id.starts_with(\"eu.\")"),
        (
            "src/providers/bedrock.rs",
            "strip_prefix(\"arn:aws:bedrock:\")",
        ),
    ];
    let probes = [
        "model.starts_with",
        "model.contains",
        "model.ends_with",
        "model.strip_prefix",
        "model ==",
        "model !=",
        "short.starts_with",
        "short.contains",
        "short.ends_with",
        "bare.starts_with",
        "bare.contains",
        "bare.ends_with",
        "model_id.starts_with",
        "model_id.contains",
        "model_id.ends_with",
        "model_id.strip_prefix",
    ];
    let mut violations: Vec<String> = Vec::new();
    let mut allowed_seen = vec![false; allowed.len()];
    for file in files {
        let text = std::fs::read_to_string(root.join(file))
            .unwrap_or_else(|_| panic!("test reads {file}"));
        for (n, line) in text.lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with("//") {
                continue; // doc/example text is not a branch
            }
            if !probes.iter().any(|p| line.contains(p)) {
                continue;
            }
            let mut is_allowed = false;
            for (i, (afile, needle)) in allowed.iter().enumerate() {
                if *afile == file && line.contains(needle) {
                    allowed_seen[i] = true;
                    is_allowed = true;
                }
            }
            if !is_allowed {
                violations.push(format!("{file}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    for (i, (_, needle)) in allowed.iter().enumerate() {
        assert!(
            allowed_seen[i],
            "allowlisted probe {needle:?} vanished — drop it from the allowlist, do not widen it"
        );
    }
    assert!(
        violations.is_empty(),
        "model-id behavior branch(es) in the reasoning path (route via ModelInfo::reasoning_shape instead):\n{}",
        violations.join("\n")
    );
}

/// OpenRouter rows nest the effort (`reasoning: {{effort}}`): the old
/// substring branch keyed on an `openrouter/` id prefix that no catalog row
/// carries, so every OpenRouter row silently took the default branch. The
/// family table keys on the provider instead.
#[test]
fn openrouter_catalog_rows_nest_effort_under_reasoning() {
    let body = compat_wire()
        .request_body("openai/gpt-5-mini", &ctx_at(ThinkingLevel::Medium))
        .expect("body");
    assert_eq!(
        body.pointer("/reasoning/effort").and_then(|v| v.as_str()),
        Some("medium"),
        "openrouter shape is a nested object, not a top-level knob: {body}"
    );
    assert!(
        body.get("reasoning_effort").is_none(),
        "the default branch's flat knob must not leak alongside: {body}"
    );
}

/// Native DeepSeek reasoning rows send the thinking switch plus effort.
#[test]
fn deepseek_native_rows_send_thinking_switch_plus_effort() {
    let body = compat_wire()
        .request_body("deepseek-v4-pro", &ctx_at(ThinkingLevel::High))
        .expect("body");
    assert_eq!(
        body.pointer("/thinking/type").and_then(|v| v.as_str()),
        Some("enabled")
    );
    assert_eq!(
        body.get("reasoning_effort").and_then(|v| v.as_str()),
        Some("high")
    );
}

/// Native Kimi rows that are not thinking rows send no knob at all (even
/// enabled): the old `kimi` branch only armed `thinking`-named rows.
#[test]
fn kimi_non_thinking_rows_send_no_knob() {
    let body = compat_wire()
        .request_body("kimi-k2.6", &ctx_at(ThinkingLevel::High))
        .expect("body");
    assert!(body.get("reasoning_effort").is_none(), "{body}");
    assert!(body.get("thinking").is_none(), "{body}");
    assert!(body.get("reasoning").is_none(), "{body}");
}

/// Z.ai GLM rows send the enable switch with `clear_thinking: false` plus effort.
#[test]
fn zai_glm_rows_send_clear_thinking_switch_plus_effort() {
    let body = compat_wire()
        .request_body("glm-5.2", &ctx_at(ThinkingLevel::High))
        .expect("body");
    assert_eq!(
        body.pointer("/thinking/type").and_then(|v| v.as_str()),
        Some("enabled")
    );
    assert_eq!(
        body.pointer("/thinking/clear_thinking")
            .and_then(|v| v.as_bool()),
        Some(false)
    );
    assert_eq!(
        body.get("reasoning_effort").and_then(|v| v.as_str()),
        Some("high")
    );
}

/// Magistral rows take `prompt_mode: reasoning` and nothing else.
#[test]
fn mistral_magistral_rows_use_prompt_mode() {
    let body = mistral_wire()
        .request_body("magistral-medium-latest", &ctx_at(ThinkingLevel::Medium))
        .expect("body");
    assert_eq!(
        body.get("prompt_mode").and_then(|v| v.as_str()),
        Some("reasoning")
    );
    assert!(body.get("reasoning_effort").is_none(), "{body}");
}

/// Mistral small/medium/devstral rows take `reasoning_effort`.
#[test]
fn mistral_small_rows_use_effort() {
    let body = mistral_wire()
        .request_body("mistral-small-2603", &ctx_at(ThinkingLevel::Medium))
        .expect("body");
    assert_eq!(
        body.get("reasoning_effort").and_then(|v| v.as_str()),
        Some("medium")
    );
    assert!(body.get("prompt_mode").is_none(), "{body}");
}

/// Other Mistral rows send neither knob.
#[test]
fn mistral_large_rows_send_neither_knob() {
    let body = mistral_wire()
        .request_body("mistral-large-latest", &ctx_at(ThinkingLevel::Medium))
        .expect("body");
    assert!(body.get("prompt_mode").is_none(), "{body}");
    assert!(body.get("reasoning_effort").is_none(), "{body}");
}

/// Bedrock carries one model-agnostic hint (`additionalModelRequestFields`)
/// whenever thinking is on — no family branches, before and after this wave.
#[test]
fn bedrock_thinking_hint_is_model_agnostic() {
    let body = BedrockConverseWire
        .request_body(
            "bedrock/anthropic.claude-sonnet-4-5",
            &ctx_at(ThinkingLevel::Medium),
        )
        .expect("body");
    assert_eq!(
        body.pointer("/additionalModelRequestFields/reasoningEffort")
            .and_then(|v| v.as_str()),
        Some("medium"),
        "{body}"
    );
    let off = BedrockConverseWire
        .request_body(
            "bedrock/anthropic.claude-sonnet-4-5",
            &ctx_at(ThinkingLevel::Off),
        )
        .expect("body");
    assert!(off.get("additionalModelRequestFields").is_none(), "{off}");
}

/// Copilot's OpenAI-dialect rows send the standard flat knob.
#[test]
fn copilot_compat_rows_send_standard_effort() {
    let body = copilot_compat_wire()
        .request_body("github-copilot/gpt-5.4", &ctx_at(ThinkingLevel::Medium))
        .expect("body");
    assert_eq!(
        body.get("reasoning_effort").and_then(|v| v.as_str()),
        Some("medium"),
        "{body}"
    );
}

/// Unknown/custom ids keep today's default-branch bytes (documented, not
/// silently changed): the flat knob when enabled, nothing when off.
#[test]
fn unknown_ids_keep_the_default_branch() {
    let body = compat_wire()
        .request_body("lmstudio/some-custom-model", &ctx_at(ThinkingLevel::Medium))
        .expect("body");
    assert_eq!(
        body.get("reasoning_effort").and_then(|v| v.as_str()),
        Some("medium"),
        "{body}"
    );
    assert!(body.get("reasoning").is_none(), "{body}");
    assert!(body.get("thinking").is_none(), "{body}");

    let off = compat_wire()
        .request_body("openai/gpt-5-mini", &ctx_at(ThinkingLevel::Off))
        .expect("body");
    assert!(off.get("reasoning").is_none(), "{off}");
    assert!(off.get("reasoning_effort").is_none(), "{off}");
}
