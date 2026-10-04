//! How refusals reach a caller: the API's envelope, the gateway's bare
//! answers, everything a proxy might send instead, and no answer at all.

use std::time::Duration;

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use gitloom::{Client, Error};

const LEAK: &str = "gl_test_LEAKPROBE_9x7q";

async fn answering(response: ResponseTemplate) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/memories"))
        .respond_with(response)
        .mount(&s)
        .await;
    s
}

async fn failure(response: ResponseTemplate) -> Error {
    let s = answering(response).await;
    let client = Client::new(LEAK).with_base_url(s.uri());
    client.get("facts/a.md", None).await.unwrap_err()
}

async fn refusal(response: ResponseTemplate) -> (u16, String, String) {
    match failure(response).await {
        Error::Api {
            status,
            code,
            message,
            ..
        } => (status, code, message),
        other => panic!("expected an API error, got {other:?}"),
    }
}

fn closed_port() -> String {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    format!("http://127.0.0.1:{port}")
}

fn assert_absent(err: &Error, secret: &str) {
    let mut seen = vec![err.to_string(), format!("{err:?}"), format!("{err:#?}")];
    let mut cause = std::error::Error::source(err);
    while let Some(c) = cause {
        seen.push(c.to_string());
        seen.push(format!("{c:?}"));
        cause = c.source();
    }
    for s in &seen {
        assert!(!s.contains(secret), "the key leaked: {s}");
    }
}

fn invalid_key(err: &Error) {
    match err {
        Error::Api {
            status,
            code,
            message,
            retry_after,
        } => {
            assert_eq!(
                (*status, code.as_str(), *retry_after),
                (0, "invalid_api_key", None)
            );
            assert_eq!(
                message,
                "The API key contains whitespace or control characters — check \
                 GITLOOM_API_KEY, or the key passed to the client."
            );
        }
        other => panic!("expected invalid_api_key, got {other:?}"),
    }
}

#[tokio::test]
async fn the_gateways_bad_key_is_unauthorized() {
    let got =
        refusal(ResponseTemplate::new(403).set_body_json(json!({"message": "Forbidden"}))).await;
    assert_eq!(
        got,
        (
            403,
            "unauthorized".into(),
            "The API key was not accepted (403 Forbidden) — check the API key \
             (GITLOOM_API_KEY, or the key passed to the client), or whether it has been revoked."
                .into()
        )
    );
}

#[tokio::test]
async fn the_gateways_missing_key_is_unauthorized() {
    let got =
        refusal(ResponseTemplate::new(401).set_body_json(json!({"message": "Unauthorized"}))).await;
    assert_eq!(
        got,
        (
            401,
            "unauthorized".into(),
            "No API key was accepted (401 Unauthorized) — check the API key \
             (GITLOOM_API_KEY, or the key passed to the client)."
                .into()
        )
    );
}

#[tokio::test]
async fn an_enveloped_refusal_keeps_its_code_and_message() {
    let got = refusal(ResponseTemplate::new(403).set_body_json(json!({"error": {
        "code": "forbidden_namespace",
        "message": "this key is scoped to another namespace"
    }})))
    .await;
    assert_eq!(
        got,
        (
            403,
            "forbidden_namespace".into(),
            "this key is scoped to another namespace".into()
        )
    );

    let got =
        refusal(ResponseTemplate::new(404).set_body_json(json!({"error": {"code": "not_found"}})))
            .await;
    assert_eq!(got, (404, "not_found".into(), "Not Found".into()));
}

#[tokio::test]
async fn a_text_error_is_its_status_and_its_text() {
    let got = refusal(ResponseTemplate::new(500).set_body_string("  upstream exploded\n")).await;
    assert_eq!(got, (500, "http_500".into(), "upstream exploded".into()));

    let (_, _, message) =
        refusal(ResponseTemplate::new(502).set_body_string("x".repeat(1000))).await;
    assert_eq!(message, format!("{}…", "x".repeat(300)));
}

#[tokio::test]
async fn a_body_with_nothing_to_say_reads_the_status() {
    for (status, body, reason) in [
        (502, "", "Bad Gateway"),
        (503, " \n\t ", "Service Unavailable"),
        (500, "null", "Internal Server Error"),
    ] {
        let got =
            refusal(ResponseTemplate::new(status).set_body_raw(body, "application/json")).await;
        assert_eq!(
            got,
            (status, format!("http_{status}"), reason.into()),
            "{body:?}"
        );
    }
}

#[tokio::test]
async fn json_that_is_not_an_object_is_shown_as_its_text() {
    for body in ["[1,2]", r#""oops""#, "42"] {
        let got = refusal(ResponseTemplate::new(400).set_body_raw(body, "application/json")).await;
        assert_eq!(got, (400, "http_400".into(), body.into()));
    }

    let got =
        refusal(ResponseTemplate::new(429).set_body_json(json!({"message": "Too Many Requests"})))
            .await;
    assert_eq!(got, (429, "http_429".into(), "Too Many Requests".into()));
}

#[tokio::test]
async fn the_legacy_flat_shape_reads_its_text() {
    let got = refusal(
        ResponseTemplate::new(500).set_body_json(json!({"error": "the index is rebuilding"})),
    )
    .await;
    assert_eq!(
        got,
        (500, "http_500".into(), "the index is rebuilding".into())
    );
}

#[tokio::test]
async fn a_rate_limit_says_when_to_try_again() {
    let limited = || {
        ResponseTemplate::new(429).set_body_json(json!({"error": {
            "code": "rate_limited", "message": "slow down"
        }}))
    };
    let retry_after = |e: Error| match e {
        Error::Api {
            code, retry_after, ..
        } => (code, retry_after),
        other => panic!("expected an API error, got {other:?}"),
    };

    let got = retry_after(failure(limited().insert_header("Retry-After", "30")).await);
    assert_eq!(got, ("rate_limited".into(), Some(Duration::from_secs(30))));
    let got = retry_after(failure(limited()).await);
    assert_eq!(got, ("rate_limited".into(), None));
    // Delta-seconds is digits alone, as the other SDKs read it.
    for header in [
        "Wed, 21 Oct 2026 07:28:00 GMT",
        "+30",
        "-30",
        "30.5",
        "3e1",
        "",
        "99999999999999999999999",
    ] {
        let got = retry_after(failure(limited().insert_header("Retry-After", header)).await);
        assert_eq!(got.1, None, "{header:?}");
    }
    let got =
        retry_after(failure(ResponseTemplate::new(503).insert_header("Retry-After", "30")).await);
    assert_eq!(got, ("http_503".into(), None));
}

#[tokio::test]
async fn no_key_fails_before_any_request() {
    let s =
        answering(ResponseTemplate::new(200).set_body_json(json!({"path": "facts/a.md"}))).await;
    let missing = |res: Result<gitloom::StoredMemory, Error>| match res {
        Err(Error::Api {
            status,
            code,
            message,
            retry_after,
        }) => {
            assert_eq!(
                (status, code.as_str(), retry_after),
                (0, "missing_api_key", None)
            );
            assert_eq!(
                message,
                "No API key. Pass it to the builder or set GITLOOM_API_KEY."
            );
        }
        other => panic!("expected missing_api_key, got {other:?}"),
    };

    std::env::remove_var("GITLOOM_API_KEY");
    for key in ["", "   ", "\t\n"] {
        let client = Client::new(key).with_base_url(s.uri());
        missing(client.get("facts/a.md", None).await);
    }
    std::env::set_var("GITLOOM_API_KEY", "  ");
    let client = Client::new("").with_base_url(s.uri());
    std::env::remove_var("GITLOOM_API_KEY");
    missing(client.get("facts/a.md", None).await);
    assert!(s.received_requests().await.unwrap().is_empty());

    std::env::set_var("GITLOOM_API_KEY", " gl_from_env\n");
    let client = Client::new(" ").with_base_url(s.uri());
    std::env::remove_var("GITLOOM_API_KEY");
    client.get("facts/a.md", None).await.unwrap();
    let sent = s.received_requests().await.unwrap();
    assert_eq!(sent[0].headers["authorization"], "Bearer gl_from_env");
}

#[tokio::test]
async fn an_unreachable_server_is_a_network_error() {
    let client = Client::new(LEAK).with_base_url(closed_port());
    let err = client.get("facts/a.md", None).await.unwrap_err();
    match &err {
        Error::Transport(e) => assert!(!e.is_timeout()),
        other => panic!("expected a transport error, got {other:?}"),
    }
    assert!(
        err.to_string().starts_with("gitloom: network error: "),
        "{err}"
    );
}

#[tokio::test]
async fn a_slow_server_is_a_timeout() {
    let s = answering(
        ResponseTemplate::new(200)
            .set_body_json(json!({"path": "facts/a.md"}))
            .set_delay(Duration::from_secs(5)),
    )
    .await;
    let client = Client::new(LEAK)
        .with_base_url(s.uri())
        .with_timeout(Duration::from_millis(100));
    let err = client.get("facts/a.md", None).await.unwrap_err();
    match &err {
        Error::Transport(e) => assert!(e.is_timeout()),
        other => panic!("expected a transport error, got {other:?}"),
    }
    assert!(err.to_string().starts_with("gitloom: timed out: "), "{err}");
}

#[tokio::test]
async fn the_default_timeout_outlasts_a_short_stall() {
    let s = answering(
        ResponseTemplate::new(200)
            .set_body_json(json!({"path": "facts/a.md"}))
            .set_delay(Duration::from_millis(1500)),
    )
    .await;
    let client = || Client::new(LEAK).with_base_url(s.uri());

    let (default, unbounded, short) = tokio::join!(
        async { client().get("facts/a.md", None).await },
        async { client().with_timeout(None).get("facts/a.md", None).await },
        async {
            client()
                .with_timeout(Duration::from_millis(200))
                .get("facts/a.md", None)
                .await
        },
    );
    assert_eq!(default.unwrap().path, "facts/a.md");
    assert_eq!(unbounded.unwrap().path, "facts/a.md");
    assert!(matches!(short, Err(Error::Transport(e)) if e.is_timeout()));
}

#[tokio::test]
async fn the_key_is_trimmed_where_the_client_is_built() {
    let s =
        answering(ResponseTemplate::new(200).set_body_json(json!({"path": "facts/a.md"}))).await;
    Client::new("glk_SECRET\n")
        .with_base_url(s.uri())
        .get("facts/a.md", None)
        .await
        .unwrap();
    let sent = s.received_requests().await.unwrap();
    assert_eq!(sent[0].headers["authorization"], "Bearer glk_SECRET");
}

#[tokio::test]
async fn a_key_with_whitespace_or_control_characters_is_refused_before_sending() {
    let s =
        answering(ResponseTemplate::new(200).set_body_json(json!({"path": "facts/a.md"}))).await;
    for key in [
        "glk_SEC\r\nRET",
        "glk_SEC RET",
        "glk_SEC\tRET",
        "glk_SEC\u{7f}RET",
        "glk_SECRÉT",
    ] {
        let err = Client::new(key)
            .with_base_url(s.uri())
            .get("facts/a.md", None)
            .await
            .unwrap_err();
        invalid_key(&err);
        assert_absent(&err, "SEC");
    }
    assert!(s.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn the_key_never_appears_in_an_error() {
    let slow = answering(ResponseTemplate::new(200).set_delay(Duration::from_secs(5))).await;
    let forbidden =
        answering(ResponseTemplate::new(403).set_body_json(json!({"message": "Forbidden"}))).await;
    let unreachable = closed_port();

    for (key, secret) in [
        (LEAK, LEAK),
        ("glk_SECRET\n", "glk_SECRET"),
        ("glk_SEC\r\nRET", "SEC"),
    ] {
        let at = |base: String| {
            Client::new(key)
                .with_base_url(base)
                .with_timeout(Duration::from_millis(100))
        };
        let refused = at(forbidden.uri())
            .get("facts/a.md", None)
            .await
            .unwrap_err();
        let lost = at(unreachable.clone())
            .get("facts/a.md", None)
            .await
            .unwrap_err();
        let timed_out = at(slow.uri()).get("facts/a.md", None).await.unwrap_err();

        if secret == "SEC" {
            for err in [&refused, &lost, &timed_out] {
                invalid_key(err);
            }
        } else {
            assert!(
                matches!(refused, Error::Api { status: 403, .. }),
                "{refused:?}"
            );
            assert!(
                matches!(&lost, Error::Transport(e) if !e.is_timeout()),
                "{lost:?}"
            );
            assert!(std::error::Error::source(&lost).is_some());
            assert!(
                matches!(&timed_out, Error::Transport(e) if e.is_timeout()),
                "{timed_out:?}"
            );
        }
        for err in [&refused, &lost, &timed_out] {
            assert_absent(err, secret);
        }
    }
    assert!(forbidden.received_requests().await.unwrap().len() == 2);
}
