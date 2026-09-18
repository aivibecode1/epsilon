//! Hand-maintained OpenAPI 3.1 description of the HTTP API.
//!
//! ADR-011 promised OpenAPI-from-types; the honest v1 is a checked-in spec whose path
//! set is guarded by a drift test against the router's single route table
//! (`crate::route_table`): a path registered without a spec entry, or described without
//! a registered route, fails `spec_paths_match_router`. That test compares path *sets*
//! only — methods, handlers, parameters and the schemas below the path level remain
//! hand-maintained. Codegen (utoipa) stays out to keep the dependency tree lean —
//! revisit if drift ever becomes a maintenance problem.

/// OpenAPI JSON for `GET /openapi.json`.
pub fn spec() -> serde_json::Value {
    serde_json::json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Epsilon serve API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "One session runtime exposed over HTTP/SSE (ADR-011). Events are AgentEvent JSON frames; approvals mirror the TUI dialog."
        },
        "paths": {
            "/healthz": { "get": {
                "summary": "Liveness",
                "responses": { "200": { "description": "OK", "content": { "application/json": { "schema": { "type": "object", "properties": { "ok": { "type": "boolean" } } } } } } }
            }},
            "/openapi.json": { "get": {
                "summary": "This document (exempt from the bearer token, like /healthz)",
                "responses": { "200": { "description": "OK", "content": { "application/json": { "schema": { "type": "object" } } } } }
            }},
            "/prompt": { "post": {
                "summary": "Start a run with a user task",
                "requestBody": { "required": true, "content": { "application/json": { "schema": prompt_body() } } },
                "responses": {
                    "202": { "description": "Run started; JSON body echoes started=true and the request_id when one was supplied" },
                    "400": { "description": "Empty task" },
                    "409": { "description": "Another run is already active; JSON body echoes started=false and the request_id when one was supplied" },
                    "422": { "description": "Malformed body" }
                }
            }},
            "/steer": { "post": steer_op("Queue a mid-run interjection (delivered after the current turn)") },
            "/followup": { "post": steer_op("Queue work for after the agent would stop") },
            "/abort": { "post": {
                "summary": "Cancel the active run (202) or report that none is running (409)",
                "requestBody": { "required": false, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/RequestIdBody" } } } },
                "responses": {
                    "202": { "description": "A run was active and its cancellation was delivered; JSON body echoes aborted=true and the request_id when one was supplied" },
                    "409": { "description": "No run is active; JSON body echoes aborted=false and the request_id when one was supplied" }
                }
            }},
            "/state": { "get": {
                "summary": "Live session state (the payload the stdio RPC's get_state replies with)",
                "responses": { "200": { "description": "OK", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SessionState" } } } } }
            }},
            "/events": { "get": {
                "summary": "SSE stream of AgentEvents",
                "responses": { "200": { "description": "text/event-stream; each data frame is an AgentEvent JSON object", "content": { "text/event-stream": {} } } }
            }},
            "/approvals": { "get": {
                "summary": "Pending permission asks, oldest first",
                "responses": { "200": { "description": "OK", "content": { "application/json": { "schema": { "type": "object", "properties": { "pending": { "type": "array", "items": { "$ref": "#/components/schemas/PermissionRequest" } } } } } } } }
            }},
            "/approvals/{request_id}": { "post": {
                "summary": "Answer one pending ask",
                "parameters": [ { "name": "request_id", "in": "path", "required": true, "schema": { "type": "string" } } ],
                "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/PermissionDecision" } } } },
                "responses": { "200": { "description": "Resolved" }, "404": { "description": "Unknown or already resolved" } }
            }}
        },
        "components": {
            "schemas": {
                "PromptBody": prompt_body(),
                "TextBody": { "type": "object", "required": ["text"], "properties": { "text": { "type": "string" } } },
                "RequestIdBody": { "type": "object", "properties": { "request_id": { "type": "string" } } },
                "SessionState": { "type": "object",
                    "required": ["model", "thinking_level", "is_streaming", "message_count", "pending_message_count", "session_file", "estimated_context_tokens"],
                    "properties": {
                        "model": { "type": "string" },
                        "thinking_level": { "type": "string", "enum": thinking_level_enum() },
                        "is_streaming": { "type": "boolean" },
                        "message_count": { "type": "integer" },
                        "pending_message_count": { "type": "integer" },
                        "session_file": { "type": ["string", "null"] },
                        "estimated_context_tokens": { "type": "integer" }
                    } },
                "PermissionRequest": { "type": "object", "required": ["request_id", "capability", "pattern"],
                    "properties": {
                        "request_id": { "type": "string" },
                        "capability": { "type": "string" },
                        "pattern": { "type": "string" },
                        "detail": {}
                    } },
                "PermissionDecision": { "oneOf": [
                    { "type": "object", "required": ["type"], "properties": { "type": { "const": "allow_once" } } },
                    { "type": "object", "required": ["type"], "properties": { "type": { "const": "always_session" } } },
                    { "type": "object", "required": ["type"], "properties": { "type": { "const": "deny" } } },
                    { "type": "object", "required": ["type", "guidance"], "properties": { "type": { "const": "deny_with_guidance" }, "guidance": { "type": "string" } } }
                ] }
            }
        }
    })
}

fn prompt_body() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["task"],
        "properties": {
            "task": { "type": "string" },
            "request_id": { "type": "string" }
        }
    })
}

fn steer_op(summary: &str) -> serde_json::Value {
    serde_json::json!({
        "post": {
            "summary": summary,
            "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TextBody" } } } },
            "responses": { "202": { "description": "Queued" }, "422": { "description": "Malformed body" } }
        }
    })
}

/// The thinking-level vocabulary as this API *emits* it.
///
/// `SessionState` is a **response** schema (`GET /state`), so this enum must list exactly
/// what serde writes — [`ThinkingLevel::XHigh`]'s canonical `x_high`, the
/// session-persistence spelling — and not the additional spelling accepted on *input*
/// (pi's `xhigh` alias; aliases never serialize). Derived from the type itself, so a
/// future rename cannot leave this document declaring a value the server never sends.
/// This document has no request-side schema carrying a thinking level today; if one is
/// added, its enum must instead list the accepted spellings (`x_high` *and* `xhigh`).
fn thinking_level_enum() -> serde_json::Value {
    serde_json::Value::Array(
        epsilon_contracts::ThinkingLevel::ALL
            .iter()
            .map(|level| serde_json::to_value(level).expect("ThinkingLevel serializes"))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The guard that keeps the hand-maintained document honest: the spec's path *set*
    /// must equal the router's own `route_table()` — the single place routes are
    /// registered (`crate::route_table`, folded into `router()`). A path added to either
    /// side alone fails here.
    ///
    /// What it does NOT compare: methods, handlers, parameters, request/response schemas
    /// or `$ref`s — those stay hand-maintained below the path level.
    #[test]
    fn spec_paths_match_router() {
        let spec = spec();

        // Well-formed JSON with the schema names this document promises.
        assert!(spec.is_object(), "spec is not a JSON object: {spec}");
        let schemas = spec["components"]["schemas"]
            .as_object()
            .expect("components.schemas must be an object");
        for name in [
            "PromptBody",
            "TextBody",
            "RequestIdBody",
            "SessionState",
            "PermissionRequest",
            "PermissionDecision",
        ] {
            assert!(schemas.contains_key(name), "spec missing schema {name}");
        }

        let mut spec_paths: Vec<&str> = spec["paths"]
            .as_object()
            .expect("paths must be an object")
            .keys()
            .map(|k| k.as_str())
            .collect();
        let mut route_paths: Vec<&str> = crate::route_table()
            .into_iter()
            .map(|(path, _)| path)
            .collect();
        spec_paths.sort_unstable();
        route_paths.sort_unstable();
        assert_eq!(
            spec_paths, route_paths,
            "spec paths must equal the routes `router()` mounts"
        );
    }

    /// The `SessionState.thinking_level` enum declares what the API emits (serde's
    /// serialization set), not what it accepts: `x_high` is in; pi's input-only alias
    /// `xhigh` is out.
    #[test]
    fn thinking_level_enum_matches_what_is_emitted() {
        let spec = spec();
        let declared =
            spec["components"]["schemas"]["SessionState"]["properties"]["thinking_level"]["enum"]
                .clone();
        let emitted: Vec<serde_json::Value> = epsilon_contracts::ThinkingLevel::ALL
            .iter()
            .map(|level| serde_json::to_value(level).unwrap())
            .collect();
        assert_eq!(declared, serde_json::Value::Array(emitted));
        let declared = declared.as_array().unwrap();
        assert!(
            declared.contains(&serde_json::json!("x_high")),
            "the persistence spelling must be declared: {declared:?}"
        );
        assert!(
            !declared.contains(&serde_json::json!("xhigh")),
            "pi's input-only alias must not be offered as an output value: {declared:?}"
        );
    }

    #[test]
    fn decision_schema_covers_all_four_variants() {
        let spec = spec();
        let variants = spec["components"]["schemas"]["PermissionDecision"]["oneOf"]
            .as_array()
            .unwrap();
        assert_eq!(variants.len(), 4);
    }
}
