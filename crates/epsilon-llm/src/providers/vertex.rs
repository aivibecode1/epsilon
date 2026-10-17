//! Google Vertex AI (`generateContent` via Cloud auth, pi port).
//!
//! Ported from pi-0.84.2 `packages/ai/src/api/google-vertex.ts`: the request
//! body, SSE frame walk, and stop vocabulary are IDENTICAL to the Generative
//! Language wire (pi's Vertex chunk loop walks the same
//! `candidates[0].content.parts`; the `@google/genai` response type is
//! shared). The only deltas are endpoint + auth:
//!
//! - endpoint: `https://{location}-aiplatform.googleapis.com/v1/publishers/
//!   google/models/{model}:streamGenerateContent?alt=sse` (pi
//!   `VERTEX_BASE_URL` = `https://{location}-aiplatform.googleapis.com`);
//! - auth: `Authorization: Bearer <token>` where the token is EITHER the
//!   stored/api key verbatim (pi's API-key path: `createClientWithApiKey`
//!   sends the key as the bearer token) OR — when the key is a placeholder
//!   (`<...>`, the `gcp-vertex-credentials` marker, or empty) — an OAuth
//!   access token minted from ADC/service-account credentials via the
//!   `GOOGLE_APPLICATION_CREDENTIALS` file (pi's `createClient` +
//!   `buildGoogleAuthOptions`).
//!
//! What we deliberately do NOT port: the `@google/genai` SDK itself (we speak
//! the same REST shape directly, like every other wire here); `onPayload`
//! interception; `retryGoogleRequest` (retry lives in `crate::retry`);
//! Gemini-3 thinking-budget tables (`getGemini3ThinkingLevel`,
//! `getGoogleBudget` — our ladder maps directly via `google_level`).

use super::google::{google_on_event, google_request_body};
use super::{HttpEventStream, ProviderWire};
use epsilon_contracts::message::StopReason;
use epsilon_contracts::stream::StreamEvent;

/// Vertex wire descriptor (fixture/test visibility).
pub struct GoogleVertexWire {
    /// GCP project id (pi `resolveProject`: explicit option, then
    /// `GOOGLE_CLOUD_PROJECT` / `GCLOUD_PROJECT`).
    pub project: String,
    /// GCP location, e.g. `us-central1` (pi `resolveLocation`).
    pub location: String,
    /// Service-account / ADC credentials file (pi `buildGoogleAuthOptions`:
    /// `GOOGLE_APPLICATION_CREDENTIALS`). `None` = default ADC path.
    pub credentials_file: Option<String>,
}

impl GoogleVertexWire {
    /// Placeholder keys pi treats as "no real key, use ADC instead" (pi
    /// `resolveApiKey` + `isPlaceholderApiKey`).
    pub fn is_placeholder_key(key: &str) -> bool {
        let k = key.trim();
        k.is_empty()
            || k == "gcp-vertex-credentials"
            || (k.starts_with('<') && k.ends_with('>') && k.len() >= 2)
    }
}

/// Where the Vertex bearer token comes from (pi `resolveApiKey` fork).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BearerSource<'a> {
    /// Send the stored/API key verbatim as the bearer token.
    KeyDirect(&'a str),
    /// Mint an OAuth token from ADC (caller runs `adc_access_token`).
    MintFromAdc,
}

/// Parsed service-account identity for the JWT-bearer grant.
#[derive(Debug, Clone)]
pub struct AdcJwtInput {
    /// `client_email` from the service-account JSON.
    pub client_email: String,
    /// PEM `private_key` from the service-account JSON (RS256 signs the grant).
    pub private_key: String,
}

impl AdcJwtInput {
    /// OAuth scope pi's SDK requests for Vertex (cloud-platform covers
    /// `aiplatform.googleapis.com`).
    pub fn scope(&self) -> &'static str {
        "https://www.googleapis.com/auth/cloud-platform"
    }

    /// Token endpoint for the JWT-bearer grant.
    pub fn token_url(&self) -> &'static str {
        "https://oauth2.googleapis.com/token"
    }
}

impl ProviderWire for GoogleVertexWire {
    fn endpoint(&self, model: &str) -> String {
        // Catalog ids are provider-qualified (`vertex/gemini-2.5-flash` —
        // uniqueness against the API-key `google` rows); the publisher path
        // takes the bare model name.
        let bare = model.split_once('/').map(|(_, rest)| rest).unwrap_or(model);
        format!(
            "https://{}-aiplatform.googleapis.com/v1/publishers/google/models/{}:streamGenerateContent?alt=sse",
            self.location, bare
        )
    }

    fn env_key(&self) -> &str {
        // The Vertex rows carry the project/location in the stored
        // credential's `env` map (pi's login stores them there); the KEY slot
        // doubles as the API-key-or-placeholder bearer slot.
        "GOOGLE_CLOUD_API_KEY"
    }

    fn headers(&self, key: &str) -> Vec<(String, String)> {
        // The key here is the RESOLVED bearer token (direct key or minted
        // ADC token); resolving it is not this layer's job.
        vec![("Authorization".to_string(), format!("Bearer {key}"))]
    }

    fn request_body(
        &self,
        model: &str,
        ctx: &crate::LlmContext,
    ) -> Result<serde_json::Value, String> {
        Ok(google_request_body(model, ctx))
    }

    fn on_event(&self, ev: &super::SseEvent, st: &mut HttpEventStream) -> Option<Vec<StreamEvent>> {
        google_on_event(ev, st)
    }

    fn map_stop(&self, wire: Option<&str>, has_tool_calls: bool) -> (StopReason, Option<String>) {
        super::google::google_map_stop(wire, has_tool_calls)
    }

    fn finalize_usage_from_tail(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_is_location_scoped_publisher_path() {
        let w = GoogleVertexWire {
            project: "p".into(),
            location: "us-central1".into(),
            credentials_file: None,
        };
        // Provider-qualified catalog id → bare model in the publisher path.
        assert_eq!(
            w.endpoint("vertex/gemini-2.5-flash"),
            "https://us-central1-aiplatform.googleapis.com/v1/publishers/google/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
        );
        assert_eq!(w.env_key(), "GOOGLE_CLOUD_API_KEY");
    }

    /// A placeholder key is "no real key, mint from ADC"; a real API key is not.
    ///
    /// The bearer fork that read this moved out of the wire layer, so the test pins
    /// the predicate itself: the two things a Vertex user can have in the key slot
    /// — an `AIza…` API key and the placeholder that means ADC — land on opposite
    /// sides of the one line, and a trailing space does not turn a real key into a
    /// placeholder.
    #[test]
    fn placeholder_keys_route_to_adc_real_keys_pass_through() {
        assert!(GoogleVertexWire::is_placeholder_key(""));
        assert!(GoogleVertexWire::is_placeholder_key(
            "gcp-vertex-credentials"
        ));
        assert!(GoogleVertexWire::is_placeholder_key(
            "<GOOGLE_CLOUD_API_KEY>"
        ));
        assert!(!GoogleVertexWire::is_placeholder_key("AIza-real"));
        assert!(!GoogleVertexWire::is_placeholder_key("AIza-real "));
    }

    #[test]
    fn request_body_matches_api_key_wire_shape() {
        use crate::stream_fn::LlmContext;
        use epsilon_contracts::content::UserContent;
        use epsilon_contracts::message::{Message, UserMessage};
        use epsilon_contracts::new_id;
        let ctx = LlmContext {
            system_prompt: "sys".into(),
            messages: vec![Message::User(UserMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                content: vec![UserContent::Text { text: "hi".into() }],
            })],
            tools: vec![],
            thinking_level: epsilon_contracts::ThinkingLevel::Off,
        };
        let w = GoogleVertexWire {
            project: "p".into(),
            location: "l".into(),
            credentials_file: None,
        };
        let body = w.request_body("gemini-2.5-flash", &ctx).expect("body");
        assert!(body.get("contents").is_some());
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "sys");
    }
}
