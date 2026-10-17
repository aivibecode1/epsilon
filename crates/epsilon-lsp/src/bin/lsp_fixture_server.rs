//! A language server for tests: speaks enough LSP to prove the client, and nothing more.
//!
//! It publishes one ERROR per line containing `TODO_BAD`, so a test can write a file, ask for
//! diagnostics, and assert on a report it controls. It is a real server in the only sense that
//! matters for these tests — a child process, framed JSON-RPC on stdio, asynchronous publishes —
//! which is what makes the client's handshake, document sync, wait-for-publish and broken-server
//! paths all exercised rather than mocked (the same pattern as `mcp_fixture_server`).
//!
//! Modes, selected by the first argument:
//!
//! | arg | behaviour |
//! |---|---|
//! | *(none)* | the full server described above |
//! | `--silent` | handshake, accept documents, never publish |
//! | `--refuse` | answer `initialize` with an error |
//! | `--die` | exit immediately after the handshake |
//! | `--no-symbols` | like the full server, but `documentSymbol` answers `[]` |
//!
//! Query answers (all canned, all deterministic):
//!
//! | method | answer |
//! |---|----|
//! | `textDocument/definition` | one `Location`: the requested position echoed back, so a caller
//! asserting on it proves the position survived the round trip in the units it was sent in |
//! | `textDocument/references` | 25 `Location`s — the declaration (the echo) plus lines 0–23 —
//! or 24 without the declaration when `context.includeDeclaration` is false |
//! | `textDocument/implementation` | one fixed `Location` at 0-based (0,0)–(0,5) |
//! | `textDocument/documentSymbol` | three `SymbolInformation`s (`alpha`/`beta` functions,
//! `Config` struct), or `[]` under `--no-symbols` |

use std::io::Write;

use epsilon_lsp::framing;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map(String::as_str).unwrap_or("");

    // Every start is recorded when the test asks for it, so "a broken server is not respawned on
    // every edit" is an assertion about a file rather than a hope about behaviour.
    if let Some(path) = std::env::var_os("EPSILON_LSP_FIXTURE_LOG") {
        use std::io::Write as _;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(file, "start {mode}");
        }
    }

    let stdin = tokio::io::stdin();
    let mut reader = tokio::io::BufReader::new(stdin);
    let mut stdout = tokio::io::stdout();

    if mode == "--die" {
        // A server that cannot start: the client must treat it as broken, not as "no diagnostics".
        send_initialize_error(&mut stdout, None).await;
        return;
    }

    loop {
        let message = match framing::read_message(&mut reader).await {
            Ok(Some(message)) => message,
            Ok(None) => return, // clean EOF
            Err(_) => return,   // malformed input ends the fixture, like a real crash
        };
        let method = message
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or_default()
            .to_string();

        match method.as_str() {
            "initialize" => {
                let id = message.get("id").cloned().unwrap_or(serde_json::json!(1));
                if mode == "--refuse" {
                    send_initialize_error(&mut stdout, Some(id)).await;
                    return;
                }
                let response = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "capabilities": {
                            "textDocumentSync": 1,
                            "diagnosticsProvider": true
                        },
                        "serverInfo": {"name": "lsp-fixture", "version": "0.1.0"}
                    }
                });
                write(&mut stdout, &response).await;
            }
            "initialized" => {}
            "textDocument/didOpen" | "textDocument/didChange" => {
                if mode == "--silent" {
                    continue;
                }
                let params = &message["params"];
                let uri = params["textDocument"]["uri"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let text = if method == "textDocument/didOpen" {
                    params["textDocument"]["text"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string()
                } else {
                    params["contentChanges"][0]["text"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string()
                };
                let diagnostics = diagnose(&text);
                let publish = serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/publishDiagnostics",
                    "params": {"uri": uri, "diagnostics": diagnostics}
                });
                write(&mut stdout, &publish).await;
            }
            "textDocument/definition" => {
                let id = message.get("id").cloned().unwrap_or(serde_json::json!(1));
                let params = &message["params"];
                let uri = params["textDocument"]["uri"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let line = params["position"]["line"].clone();
                let character = params["position"]["character"].clone();
                write(
                    &mut stdout,
                    &serde_json::json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {
                            "uri": uri,
                            "range": {
                                "start": {"line": line.clone(), "character": character.clone()},
                                "end": {"line": line, "character": character}
                            }
                        }
                    }),
                )
                .await;
            }
            "textDocument/references" => {
                let id = message.get("id").cloned().unwrap_or(serde_json::json!(1));
                let params = &message["params"];
                let uri = params["textDocument"]["uri"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let include = params["context"]["includeDeclaration"]
                    .as_bool()
                    .unwrap_or(true);
                let mut hits = Vec::new();
                if include {
                    hits.push(serde_json::json!({
                        "uri": uri,
                        "range": {
                            "start": {
                                "line": params["position"]["line"].clone(),
                                "character": params["position"]["character"].clone()
                            },
                            "end": {
                                "line": params["position"]["line"].clone(),
                                "character": params["position"]["character"].clone()
                            }
                        }
                    }));
                }
                for i in 0..24 {
                    hits.push(serde_json::json!({
                        "uri": uri,
                        "range": {
                            "start": {"line": i, "character": 0},
                            "end": {"line": i, "character": 1}
                        }
                    }));
                }
                write(
                    &mut stdout,
                    &serde_json::json!({"jsonrpc": "2.0", "id": id, "result": hits}),
                )
                .await;
            }
            "textDocument/implementation" => {
                let id = message.get("id").cloned().unwrap_or(serde_json::json!(1));
                let uri = message["params"]["textDocument"]["uri"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                write(
                    &mut stdout,
                    &serde_json::json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": [{
                            "uri": uri,
                            "range": {
                                "start": {"line": 0, "character": 0},
                                "end": {"line": 0, "character": 5}
                            }
                        }]
                    }),
                )
                .await;
            }
            "textDocument/documentSymbol" => {
                let id = message.get("id").cloned().unwrap_or(serde_json::json!(1));
                if mode == "--no-symbols" {
                    write(
                        &mut stdout,
                        &serde_json::json!({"jsonrpc": "2.0", "id": id, "result": []}),
                    )
                    .await;
                    continue;
                }
                let uri = message["params"]["textDocument"]["uri"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let symbols = ["alpha", "beta"]
                    .iter()
                    .enumerate()
                    .map(|(i, name)| {
                        serde_json::json!({
                            "name": name,
                            "kind": "Function",
                            "location": {
                                "uri": uri,
                                "range": {
                                    "start": {"line": i, "character": 0},
                                    "end": {"line": i, "character": 4}
                                }
                            }
                        })
                    })
                    .chain(std::iter::once(serde_json::json!({
                        "name": "Config",
                        "kind": "Struct",
                        "location": {
                            "uri": uri,
                            "range": {
                                "start": {"line": 2, "character": 0},
                                "end": {"line": 2, "character": 6}
                            }
                        }
                    })))
                    .collect::<Vec<_>>();
                write(
                    &mut stdout,
                    &serde_json::json!({"jsonrpc": "2.0", "id": id, "result": symbols}),
                )
                .await;
            }
            "shutdown" => {
                let id = message.get("id").cloned().unwrap_or(serde_json::json!(1));
                write(
                    &mut stdout,
                    &serde_json::json!({"jsonrpc": "2.0", "id": id, "result": null}),
                )
                .await;
            }
            "exit" => return,
            _ => {
                // Unknown request: answer `null` so a client waiting on an id never hangs.
                if let Some(id) = message.get("id") {
                    write(
                        &mut stdout,
                        &serde_json::json!({"jsonrpc": "2.0", "id": id, "result": null}),
                    )
                    .await;
                }
            }
        }
    }
}

/// One ERROR for each `TODO_BAD` occurrence, at its line and column.
fn diagnose(text: &str) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if let Some(column) = line.find("TODO_BAD") {
            out.push(serde_json::json!({
                "range": {
                    "start": {"line": index, "character": column},
                    "end": {"line": index, "character": column + 8}
                },
                "severity": 1,
                "source": "lsp-fixture",
                "message": "found a TODO_BAD marker"
            }));
        }
    }
    out
}

async fn send_initialize_error(stdout: &mut tokio::io::Stdout, id: Option<serde_json::Value>) {
    let message = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(serde_json::json!(1)),
        "error": {"code": -32603, "message": "fixture refuses to initialize"}
    });
    write(stdout, &message).await;
}

async fn write(stdout: &mut tokio::io::Stdout, message: &serde_json::Value) {
    let _ = framing::write_message(stdout, message).await;
    let _ = std::io::stdout().flush();
}
