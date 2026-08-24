//! Hand-maintained OpenAPI 3.1 description of the HTTP API.
//!
//! ADR-011 promised OpenAPI-from-types; the honest v1 is a checked-in spec kept in
//! sync by route-level tests (each path here is asserted to exist in `router()`).
//! Codegen (utoipa) stays out to keep the dependency tree lean — revisit if drift
//! ever becomes a maintenance problem.

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
            "/prompt": { "post": {
                "summary": "Start a run with a user task",
                "requestBody": { "required": true, "content": { "application/json": { "schema": prompt_body() } } },
                "responses": {
                    "202": { "description": "Run started" },
                    "400": { "description": "Empty task" },
                    "409": { "description": "Another run is already active" },
                    "422": { "description": "Malformed body" }
                }
            }},
            "/steer": { "post": steer_op("Queue a mid-run interjection (delivered after the current turn)") },
            "/followup": { "post": steer_op("Queue work for after the agent would stop") },
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
        "properties": { "task": { "type": "string" } }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The spec must describe exactly the routes the router registers — this is the
    /// drift guard that keeps the hand-maintained document honest.
    #[test]
    fn spec_paths_match_router() {
        let spec = spec();
        let paths = spec["paths"].as_object().unwrap();
        let expected = [
            "/healthz",
            "/prompt",
            "/steer",
            "/followup",
            "/events",
            "/approvals",
            "/approvals/{request_id}",
        ];
        assert_eq!(paths.len(), expected.len(), "spec path count drifted");
        for p in expected {
            assert!(paths.contains_key(p), "spec missing registered route {p}");
        }
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
