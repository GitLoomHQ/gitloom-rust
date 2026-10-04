//! How refusals reach a caller: the API's envelope, the gateway's bare
//! answers, and everything a proxy might send instead.

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use gitloom::{Client, Error};

async fn refusing(response: ResponseTemplate) -> (MockServer, Client) {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/memories"))
        .respond_with(response)
        .mount(&s)
        .await;
    let client = Client::new("gl_test").with_base_url(s.uri());
    (s, client)
}

async fn refusal(response: ResponseTemplate) -> (u16, String, String) {
    let (_s, client) = refusing(response).await;
    match client.get("facts/a.md", None).await {
        Err(Error::Api {
            status,
            code,
            message,
        }) => (status, code, message),
        other => panic!("expected an API error, got {other:?}"),
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
            "The API key was not accepted (403 Forbidden) — check GITLOOM_API_KEY, \
             or whether the key has been revoked."
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
            "No API key was accepted (401 Unauthorized) — check GITLOOM_API_KEY.".into()
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

    let got = refusal(ResponseTemplate::new(502)).await;
    assert_eq!(got, (502, "http_502".into(), "Bad Gateway".into()));
}

#[tokio::test]
async fn json_that_is_not_an_envelope_does_not_misparse() {
    let got = refusal(ResponseTemplate::new(500).set_body_raw("null", "application/json")).await;
    assert_eq!(got, (500, "http_500".into(), "null".into()));

    let got = refusal(ResponseTemplate::new(400).set_body_raw("[1,2]", "application/json")).await;
    assert_eq!(got, (400, "http_400".into(), "[1,2]".into()));

    let got =
        refusal(ResponseTemplate::new(429).set_body_json(json!({"message": "Too Many Requests"})))
            .await;
    assert_eq!(got, (429, "http_429".into(), "Too Many Requests".into()));

    let got = refusal(ResponseTemplate::new(500).set_body_json(json!({"error": "flat"}))).await;
    assert_eq!(got, (500, "http_500".into(), r#"{"error":"flat"}"#.into()));
}

#[tokio::test]
async fn no_key_fails_before_any_request() {
    let (s, _) =
        refusing(ResponseTemplate::new(200).set_body_json(json!({"path": "facts/a.md"}))).await;

    std::env::remove_var("GITLOOM_API_KEY");
    let client = Client::new("").with_base_url(s.uri());
    match client.get("facts/a.md", None).await {
        Err(Error::Api {
            status,
            code,
            message,
        }) => {
            assert_eq!((status, code.as_str()), (0, "missing_api_key"));
            assert_eq!(
                message,
                "No API key. Pass it to the builder or set GITLOOM_API_KEY."
            );
        }
        other => panic!("expected missing_api_key, got {other:?}"),
    }
    assert!(s.received_requests().await.unwrap().is_empty());

    std::env::set_var("GITLOOM_API_KEY", "gl_from_env");
    let client = Client::new("").with_base_url(s.uri());
    std::env::remove_var("GITLOOM_API_KEY");
    client.get("facts/a.md", None).await.unwrap();
    let sent = s.received_requests().await.unwrap();
    assert_eq!(sent[0].headers["authorization"], "Bearer gl_from_env");
}

#[tokio::test]
async fn an_unreachable_server_is_a_network_error() {
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    let client = Client::new("gl_test").with_base_url(format!("http://127.0.0.1:{port}"));
    let err = client.get("facts/a.md", None).await.unwrap_err();
    assert!(matches!(err, Error::Transport(_)), "{err:?}");
    assert!(
        err.to_string().starts_with("gitloom: network error: "),
        "{err}"
    );
}
