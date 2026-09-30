//! OAuth login-ceremony tests that must run offline (M52).
//!
//! Everything here is pure or loopback-only: PKCE math against RFC 7636's
//! own vector, paste parsing, the callback listener against a real TCP
//! socket on 127.0.0.1, the device poller against scripted server verdicts
//! (virtual time), and the response parsers. No provider endpoint is ever
//! contacted.

use std::time::Duration;

use epsilon_llm::oauth_login::{
    CallbackListener, DevicePoll, ExchangeOutcome, LOGIN_FLOWS, LoginKind, MintKind, authorize_url,
    base64url, device_code_request_body, device_poll_body, generate_pkce, login_flow,
    oauth_providers, parse_authorization_input, parse_callback_request_line, parse_device_code,
    parse_device_poll, parse_device_poll_for, parse_exchange_response, parse_github_device_token,
    poll_device_code, s256_challenge,
};

#[test]
fn pkce_matches_rfc7636_appendix_b_vector() {
    // RFC 7636 Appendix B: verifier → challenge is the S256 contract our
    // providers verify server-side.
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    assert_eq!(
        s256_challenge(verifier),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
}

#[test]
fn generated_pkce_is_csprng_sized_base64url_and_self_consistent() {
    let first = generate_pkce().expect("os randomness");
    let second = generate_pkce().expect("os randomness");
    assert_ne!(first.verifier, second.verifier, "two draws must differ");
    // 32 bytes base64url without padding = 43 chars, inside RFC 7636's 43..128.
    assert_eq!(first.verifier.len(), 43);
    assert!(
        first
            .verifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "verifier must be base64url: {}",
        first.verifier
    );
    assert_eq!(first.challenge, s256_challenge(&first.verifier));
    assert_ne!(first.challenge, first.verifier);
}

#[test]
fn base64url_strips_padding_and_uses_url_alphabet() {
    // Bytes that produce '+' and '/' in standard base64: 0xfb 0xff.
    assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
    assert_eq!(base64url(b"hi"), "aGk");
}

#[test]
fn random_state_is_draw_from_csprng() {
    let a = epsilon_llm::oauth_login::random_state().unwrap();
    let b = epsilon_llm::oauth_login::random_state().unwrap();
    assert_ne!(a, b);
    assert_eq!(a.len(), 22, "16 bytes base64url = 22 chars");
}

#[test]
fn authorization_input_parses_all_four_shapes_pi_tolerates() {
    // 1. full redirect URL (what a working callback or address bar shows)
    let url =
        parse_authorization_input("http://localhost:53692/callback?code=abc123&state=xyz#fragment");
    assert_eq!(url.code.as_deref(), Some("abc123"));
    assert_eq!(url.state.as_deref(), Some("xyz"));

    // 2. out-of-band `code#state` (Anthropic's paste format)
    let pair = parse_authorization_input("abc123#xyz");
    assert_eq!(pair.code.as_deref(), Some("abc123"));
    assert_eq!(pair.state.as_deref(), Some("xyz"));

    // 3. bare query string
    let query = parse_authorization_input("code=abc123&state=xyz");
    assert_eq!(query.code.as_deref(), Some("abc123"));
    assert_eq!(query.state.as_deref(), Some("xyz"));

    // 4. bare code
    let code = parse_authorization_input("  abc123  ");
    assert_eq!(code.code.as_deref(), Some("abc123"));
    assert_eq!(code.state, None);

    // empty stays empty (no accidental Some(""))
    let empty = parse_authorization_input("   ");
    assert_eq!(empty.code, None);
    assert_eq!(empty.state, None);
}

#[test]
fn authorization_input_percent_decodes_and_treats_plus_as_space() {
    let parsed = parse_authorization_input("code=a%2Fb%2Bc&state=s%20t");
    assert_eq!(parsed.code.as_deref(), Some("a/b+c"));
    assert_eq!(parsed.state.as_deref(), Some("s t"));
}

#[test]
fn authorize_url_carries_every_pi_parameter_for_anthropic() {
    let flow = login_flow("anthropic").unwrap();
    let pkce = generate_pkce().unwrap();
    let url = authorize_url(
        &flow,
        "client-1",
        "http://localhost:53692/callback",
        &pkce,
        "state-1",
    );
    assert!(url.starts_with("https://claude.ai/oauth/authorize?"));
    for needle in [
        "code=true",
        "client_id=client-1",
        "response_type=code",
        "redirect_uri=http%3A%2F%2Flocalhost%3A53692%2Fcallback",
        "code_challenge_method=S256",
        "state=state-1",
    ] {
        assert!(url.contains(needle), "missing {needle} in {url}");
    }
    assert!(url.contains(&format!("code_challenge={}", pkce.challenge)));
    // scopes are space-delimited and percent-encoded, never raw spaces
    assert!(url.contains("scope=org%3Acreate_api_key"));
    assert!(!url.contains(' '));
}

#[test]
fn authorize_url_omits_state_and_scope_where_pi_does() {
    let flow = login_flow("openrouter").unwrap();
    let pkce = generate_pkce().unwrap();
    let url = authorize_url(
        &flow,
        "cid",
        "http://localhost:7777/oauth/callback/x",
        &pkce,
        "s",
    );
    assert!(url.starts_with("https://openrouter.ai/auth?"));
    assert!(!url.contains("state="), "openrouter takes no state: {url}");
    assert!(!url.contains("scope="), "openrouter takes no scope: {url}");
}

#[test]
fn callback_listener_captures_the_redirect_and_answers_the_browser() {
    let listener = CallbackListener::bind(
        "127.0.0.1",
        0,
        "/callback",
        "expected-state",
        Duration::from_secs(5),
    )
    .expect("loopback bind");
    let port = listener.port();
    assert!(port > 0, "ephemeral port assigned");

    let response = http_get(port, "/callback?code=the-code&state=expected-state");
    assert!(
        response.contains("200 OK"),
        "browser sees success: {response}"
    );
    assert!(response.contains("authentication complete"));

    let code = listener
        .recv_timeout(Duration::from_secs(5))
        .expect("captured")
        .expect("no listener error");
    assert_eq!(code.code, "the-code");
    assert_eq!(code.state, "expected-state");
}

#[test]
fn callback_listener_rejects_state_mismatch_and_keeps_listening() {
    let listener = CallbackListener::bind(
        "127.0.0.1",
        0,
        "/callback",
        "expected-state",
        Duration::from_secs(5),
    )
    .expect("loopback bind");
    let port = listener.port();

    let bad = http_get(port, "/callback?code=stolen&state=attacker-state");
    assert!(bad.contains("400"), "mismatch answered with 400: {bad}");
    assert!(bad.contains("state mismatch"));

    // The listener must still accept the genuine redirect afterwards.
    let good = http_get(port, "/callback?code=real&state=expected-state");
    assert!(good.contains("200 OK"));
    let code = listener
        .recv_timeout(Duration::from_secs(5))
        .expect("captured")
        .expect("no listener error");
    assert_eq!(code.code, "real");
}

#[test]
fn callback_listener_404s_other_routes_and_reports_timeouts() {
    let listener =
        CallbackListener::bind("127.0.0.1", 0, "/callback", "", Duration::from_millis(300))
            .expect("loopback bind");
    let port = listener.port();

    let wrong = http_get(port, "/nope?code=x");
    assert!(wrong.contains("404"), "wrong route answered 404: {wrong}");

    // No valid redirect before the deadline → the waiter reports a timeout.
    let outcome = listener
        .recv_timeout(Duration::from_secs(5))
        .expect("a verdict arrives");
    let message = outcome.expect_err("timeout is an error");
    assert!(message.contains("timed out"), "{message}");
}

#[test]
fn callback_listener_skips_state_check_when_flow_sends_none() {
    let listener =
        CallbackListener::bind("127.0.0.1", 0, "/cb", "", Duration::from_secs(5)).unwrap();
    let port = listener.port();
    let response = http_get(port, "/cb?code=only-code");
    assert!(response.contains("200 OK"));
    let code = listener
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .unwrap();
    assert_eq!(code.code, "only-code");
    assert_eq!(code.state, "");
}

#[test]
fn callback_request_line_parsing_is_strict_about_route_and_shape() {
    let ok = parse_callback_request_line("GET /callback?code=a&state=b HTTP/1.1", "/callback");
    assert_eq!(
        ok.unwrap().unwrap(),
        vec![
            ("code".to_string(), "a".to_string()),
            ("state".to_string(), "b".to_string())
        ]
    );
    let wrong_route = parse_callback_request_line("GET /elsewhere HTTP/1.1", "/callback");
    assert_eq!(wrong_route.unwrap().unwrap_err(), 404);
    let malformed = parse_callback_request_line("", "/callback");
    assert!(malformed.is_err());
}

fn http_get(port: u16, target: &str) -> String {
    use std::io::{Read as _, Write as _};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .write_all(format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
        .expect("write");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    let mut buf = String::new();
    let _ = stream.read_to_string(&mut buf);
    buf
}

#[tokio::test(start_paused = true)]
async fn device_poll_completes_after_pending_and_slow_down() {
    let mut calls = 0;
    let token = poll_device_code(None, 300, || {
        calls += 1;
        let verdict = match calls {
            1 => DevicePoll::Pending,
            2 => DevicePoll::SlowDown(None),
            _ => DevicePoll::Complete("access-token"),
        };
        async move { verdict }
    })
    .await
    .expect("completes");
    assert_eq!(token, "access-token");
    assert_eq!(calls, 3, "one poll per server verdict");
}

#[tokio::test(start_paused = true)]
async fn device_poll_honours_server_interval_on_slow_down() {
    // pi: a server-provided interval replaces the tracked one; without one,
    // the interval grows by 5s. Assert the *gaps* between polls under
    // virtual time.
    let mut stamps: Vec<tokio::time::Instant> = Vec::new();
    let mut calls = 0;
    poll_device_code(Some(2), 300, || {
        stamps.push(tokio::time::Instant::now());
        calls += 1;
        let verdict = if calls == 1 {
            DevicePoll::SlowDown(Some(9))
        } else if calls == 2 {
            DevicePoll::SlowDown(None)
        } else {
            DevicePoll::Complete(())
        };
        async move { verdict }
    })
    .await
    .expect("completes");
    assert_eq!(stamps.len(), 3);
    let first_gap = stamps[1] - stamps[0];
    let second_gap = stamps[2] - stamps[1];
    assert_eq!(
        first_gap,
        Duration::from_secs(9),
        "server interval replaces the tracked one"
    );
    assert_eq!(
        second_gap,
        Duration::from_secs(14),
        "a bare slow_down adds 5s to the current interval"
    );
}

#[tokio::test(start_paused = true)]
async fn device_poll_times_out_plainly_and_mentions_clock_drift_after_slow_down() {
    let plain = poll_device_code(Some(5), 0, || async { DevicePoll::<()>::Pending }).await;
    assert!(plain.unwrap_err().contains("timed out"));

    let mut calls = 0;
    let drifted = poll_device_code(Some(5), 8, || {
        calls += 1;
        async move {
            if calls == 1 {
                DevicePoll::SlowDown(None)
            } else {
                DevicePoll::<()>::Pending
            }
        }
    })
    .await;
    let message = drifted.unwrap_err();
    assert!(message.contains("clock drift"), "{message}");
}

#[tokio::test(start_paused = true)]
async fn device_poll_surfaces_terminal_failures_verbatim() {
    let denied = poll_device_code(None, 300, || async {
        DevicePoll::<()>::Failed("xai device authorization was denied".to_string())
    })
    .await;
    assert_eq!(denied.unwrap_err(), "xai device authorization was denied");
}

#[test]
fn device_code_response_parsing_rejects_untrusted_uris() {
    let ok = parse_device_code(
        r#"{"device_code":"dc","user_code":"AB-12","verification_uri":"https://auth.x.ai/device","expires_in":600,"interval":5}"#,
    )
    .expect("valid");
    assert_eq!(ok.user_code, "AB-12");
    assert_eq!(ok.interval_seconds, Some(5));
    assert_eq!(ok.expires_in_seconds, 600);

    // pi refuses to hand a non-http(s) scheme to the browser opener.
    let bad = parse_device_code(
        r#"{"device_code":"dc","user_code":"AB","verification_uri":"file:///etc/passwd","expires_in":600}"#,
    );
    assert!(bad.unwrap_err().contains("untrusted verification_uri"));

    let missing = parse_device_code(r#"{"device_code":"dc","expires_in":600}"#);
    assert!(missing.unwrap_err().contains("missing user_code"));

    let no_expiry = parse_device_code(
        r#"{"device_code":"dc","user_code":"AB","verification_uri":"https://x/device"}"#,
    );
    assert!(no_expiry.unwrap_err().contains("missing expires_in"));
}

#[test]
fn device_poll_response_maps_rfc8628_verdicts() {
    let flow = login_flow("xai").unwrap();
    assert_eq!(
        parse_device_poll(&flow, r#"{"error":"authorization_pending"}"#, 0),
        DevicePoll::Pending
    );
    assert_eq!(
        parse_device_poll(&flow, r#"{"error":"slow_down","interval":12}"#, 0),
        DevicePoll::SlowDown(Some(12))
    );
    assert!(matches!(
        parse_device_poll(&flow, r#"{"error":"access_denied"}"#, 0),
        DevicePoll::Failed(m) if m.contains("denied")
    ));
    assert!(matches!(
        parse_device_poll(&flow, r#"{"error":"expired_token"}"#, 0),
        DevicePoll::Failed(m) if m.contains("expired")
    ));
    let complete = parse_device_poll(
        &flow,
        r#"{"access_token":"at","refresh_token":"rt","expires_in":3600}"#,
        1_000_000,
    );
    assert_eq!(
        complete,
        DevicePoll::Complete(ExchangeOutcome::Pair {
            access: "at".to_string(),
            refresh: "rt".to_string(),
            expires: 1_000_000 + 3_600_000 - 300_000,
            env: None,
        })
    );
    assert!(matches!(
        parse_device_poll(&flow, "not json", 0),
        DevicePoll::Failed(m) if m.contains("invalid JSON")
    ));
}

#[test]
fn exchange_response_reads_pairs_and_openrouter_keys() {
    let anthropic = login_flow("anthropic").unwrap();
    let pair = parse_exchange_response(
        &anthropic,
        r#"{"access_token":"at","refresh_token":"rt","expires_in":60}"#,
        1_000_000,
    )
    .unwrap();
    assert_eq!(
        pair,
        ExchangeOutcome::Pair {
            access: "at".into(),
            refresh: "rt".into(),
            expires: 1_000_000 + 60_000 - 300_000,
            env: None,
        }
    );

    // A pair without a refresh token cannot be kept alive → refuse it.
    let no_refresh =
        parse_exchange_response(&anthropic, r#"{"access_token":"at","expires_in":60}"#, 0);
    assert!(no_refresh.unwrap_err().contains("refresh_token"));

    let openrouter = login_flow("openrouter").unwrap();
    assert_eq!(
        parse_exchange_response(&openrouter, r#"{"key":"sk-or-v1-xyz"}"#, 0).unwrap(),
        ExchangeOutcome::Key("sk-or-v1-xyz".into())
    );
    assert!(
        parse_exchange_response(&openrouter, r#"{"user_id":"u"}"#, 0)
            .unwrap_err()
            .contains("\"key\"")
    );
}

#[test]
fn login_flows_only_advertise_wires_that_exist() {
    // M55: github-copilot joined once its wire + catalog rows landed;
    // openai-codex stays absent until its model data exists (see module docs).
    let providers = oauth_providers();
    assert_eq!(
        providers,
        vec!["anthropic", "openrouter", "xai", "github-copilot"]
    );
    assert!(login_flow("openai-codex").is_none());
    let copilot = login_flow("github-copilot").unwrap();
    assert_eq!(copilot.kind, LoginKind::Device);
    assert_eq!(copilot.mint, MintKind::GitHubCopilot);
    assert_eq!(
        copilot.token_url,
        "https://github.com/login/oauth/access_token"
    );
    assert_eq!(copilot.device_url, "https://github.com/login/device/code");
    assert!(!copilot.user_agent.is_empty(), "GitHub wants a UA");
    assert!(copilot.client_id_env.ends_with("_OAUTH_CLIENT_ID"));
    for flow in LOGIN_FLOWS {
        assert_eq!(login_flow(flow.provider).unwrap().label, flow.label);
        // every flow names a client-id env var and a token endpoint
        assert!(flow.client_id_env.ends_with("_OAUTH_CLIENT_ID"));
        assert!(flow.token_url.starts_with("https://"));
        match flow.kind {
            LoginKind::Pkce => {
                assert!(!flow.authorize_url.is_empty());
                assert!(!flow.callback_path.is_empty());
            }
            LoginKind::Device => {
                assert!(!flow.device_url.is_empty());
                assert!(flow.callback_path.is_empty());
            }
        }
    }
}

#[test]
fn request_bodies_match_the_provider_conventions() {
    let xai = login_flow("xai").unwrap();
    let body = device_code_request_body(&xai, "cid");
    assert!(body.contains("client_id=cid"));
    assert!(body.contains("referrer=epsilon"));
    assert!(body.contains("grok-cli%3Aaccess"), "scope is form-encoded");

    let poll = device_poll_body(&xai, "cid", "dev-1");
    assert!(poll.contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code"));
    assert!(poll.contains("device_code=dev-1"));
}

// ---------------------------------------------------------------------------
// M55: GitHub Copilot device flow + mint
// ---------------------------------------------------------------------------

#[test]
fn github_device_poll_uses_the_standard_vocabulary_then_hands_back_a_token() {
    let flow = login_flow("github-copilot").unwrap();
    // RFC 8628 progress arms behave exactly like the other device providers.
    assert_eq!(
        parse_device_poll_for(&flow, r#"{"error":"authorization_pending"}"#, 0),
        DevicePoll::Pending
    );
    assert_eq!(
        parse_device_poll_for(&flow, r#"{"error":"slow_down","interval":10}"#, 0),
        DevicePoll::SlowDown(Some(10))
    );
    match parse_device_poll_for(&flow, r#"{"error":"access_denied"}"#, 0) {
        DevicePoll::Failed(m) => assert!(m.contains("denied"), "{m}"),
        other => panic!("expected Failed, got {other:?}"),
    }
    match parse_device_poll_for(&flow, r#"{"error":"expired_token"}"#, 0) {
        DevicePoll::Failed(m) => assert!(m.contains("expired"), "{m}"),
        other => panic!("expected Failed, got {other:?}"),
    }
    // Success is a bare GitHub token — no expiry, no refresh token — wrapped
    // for the mint step rather than stored as a pair.
    match parse_device_poll_for(
        &flow,
        r#"{"access_token":"gho_x","token_type":"bearer","scope":"read:user"}"#,
        0,
    ) {
        DevicePoll::Complete(ExchangeOutcome::Device(token)) => assert_eq!(token, "gho_x"),
        other => panic!("expected a device token, got {other:?}"),
    }
    assert!(parse_github_device_token(r#"{"token_type":"bearer"}"#).is_err());
    assert!(parse_github_device_token(r#"{"access_token":"  "}"#).is_err());
    assert!(parse_github_device_token("nope").is_err());
}

#[test]
fn mint_errors_name_the_missing_field() {
    match parse_device_poll_for(&login_flow("github-copilot").unwrap(), "not json", 0) {
        DevicePoll::Failed(m) => assert!(m.contains("invalid JSON"), "{m}"),
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn the_device_poll_parser_is_chosen_by_the_flows_mint() {
    // xAI-style flows keep the pair parser (their success body carries
    // access+refresh+expires_in); Copilot's takes the GitHub shape.
    let xai = login_flow("xai").unwrap();
    let body = r#"{"access_token":"at","refresh_token":"rt","expires_in":3600}"#;
    match parse_device_poll_for(&xai, body, 1_000_000) {
        DevicePoll::Complete(ExchangeOutcome::Pair {
            access, refresh, ..
        }) => {
            assert_eq!((access.as_str(), refresh.as_str()), ("at", "rt"));
        }
        other => panic!("expected a pair, got {other:?}"),
    }
    // The same body means different things per flow: copilot reads only the
    // bare GitHub token (`access_token`) and ignores the pair fields — which is
    // precisely why the mint step exists instead of storing this as a pair.
    match parse_device_poll_for(&login_flow("github-copilot").unwrap(), body, 0) {
        DevicePoll::Complete(ExchangeOutcome::Device(token)) => assert_eq!(token, "at"),
        other => panic!("expected the GitHub shape, got {other:?}"),
    }
}
