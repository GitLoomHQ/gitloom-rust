//! Against a fake API that keeps the server's invariants.

use serde_json::{json, Value};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use gitloom::{Client, Content, Conversation, ConversationOptions, Message, Part, Usage};

async fn server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/conversations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"branch": "main", "next_seq": 0})))
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"/v1/conversations/.+/messages$"))
        .respond_with(|req: &Request| {
            let body: Value = req.body_json().unwrap();
            let n = body["messages"].as_array().unwrap().len();
            ResponseTemplate::new(200).set_body_json(json!({"next_seq": n, "written": n}))
        })
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"/v1/conversations/.+/compact$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"compacted": true, "summary": "server summary"})))
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/media"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "med-1", "bytes": 5})))
        .mount(&s)
        .await;
    s
}

fn opts(model: &str, summarize: bool) -> ConversationOptions {
    ConversationOptions {
        model: model.into(),
        summarize: summarize.then(|| {
            Box::new(|_: &[Message]| Ok("summarized".to_string())) as gitloom::Summarizer
        }),
        ..Default::default()
    }
}

#[tokio::test]
async fn cadence_compaction_fires_with_tokens_to_spare() {
    let s = server().await;
    let client = Client::new("gl_test").with_base_url(s.uri());
    let mut o = opts("claude-sonnet-5", true);
    o.compact_every = Some(2);
    let mut conv = Conversation::create(&client, "c1", o).await.unwrap();

    for i in 0..3 {
        conv.append(
            vec![Message::user(format!("q{i}")), Message::assistant(format!("a{i}"))],
            None,
        )
        .await
        .unwrap();
    }
    let compactions = s
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path().ends_with("/compact"))
        .count();
    assert!(compactions >= 1, "the cadence never compacted; nothing would reach memory");
}

#[tokio::test]
async fn reported_usage_beats_the_estimator() {
    let s = server().await;
    let client = Client::new("gl_test").with_base_url(s.uri());
    let mut o = opts("gpt-4o", true);
    o.max_tokens = Some(10_000);
    o.compact_at = Some(0.5);
    o.compact_every = Some(0);
    let mut conv = Conversation::create(&client, "c1", o).await.unwrap();

    let usage = Usage { prompt_tokens: Some(9_000), completion_tokens: Some(500), ..Default::default() };
    conv.append(vec![Message::user("short"), Message::assistant("also short")], Some(&usage))
        .await
        .unwrap();
    conv.append(vec![Message::user("tiny")], None).await.unwrap();

    let compactions = s
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path().ends_with("/compact"))
        .count();
    assert!(compactions >= 1, "9k reported tokens against a 5k threshold did not compact");
}

#[tokio::test]
async fn data_parts_upload_and_store_references() {
    let s = server().await;
    let client = Client::new("gl_test").with_base_url(s.uri());
    let mut conv = Conversation::create(&client, "c1", opts("gpt-4o", false)).await.unwrap();

    conv.append(
        vec![Message {
            role: "user".into(),
            content: Content::Parts(vec![
                Part::text_part("look at this"),
                Part::image_data("aGVsbG8=", "image/png"),
            ]),
            ..Default::default()
        }],
        None,
    )
    .await
    .unwrap();

    let reqs = s.received_requests().await.unwrap();
    assert!(reqs.iter().any(|r| r.url.path() == "/v1/media"), "bytes were never uploaded");
    let append = reqs.iter().find(|r| r.url.path().ends_with("/messages")).unwrap();
    let body: Value = append.body_json().unwrap();
    let parts = &body["messages"][0]["parts"];
    assert_eq!(parts[1]["media_id"], "med-1");
    assert!(parts[1].get("data").is_none(), "bytes landed in the stored message");
    assert_eq!(body["messages"][0]["content"], "look at this");
}

#[tokio::test]
async fn usage_accepts_both_spellings() {
    let openai = Usage { prompt_tokens: Some(10), completion_tokens: Some(5), ..Default::default() };
    let anthropic = Usage { input_tokens: Some(10), output_tokens: Some(5), ..Default::default() };
    assert_eq!(openai.total(), 15);
    assert_eq!(anthropic.total(), 15);
}

#[tokio::test]
async fn server_side_compaction_asks_gitloom() {
    let s = server().await;
    let client = Client::new("gl_test").with_base_url(s.uri());
    let mut o = opts("claude-sonnet-5", false);
    o.summarize_server = true;
    o.compact_every = Some(1);
    let mut conv = Conversation::create(&client, "c1", o).await.unwrap();

    for _ in 0..2 {
        conv.append(vec![Message::user("q"), Message::assistant("a")], None)
            .await
            .unwrap();
    }
    let reqs = s.received_requests().await.unwrap();
    let auto = reqs.iter().find(|r| r.url.path().ends_with("/compact")).expect("no compaction");
    let body: Value = auto.body_json().unwrap();
    assert_eq!(body["auto"], true);
    assert!(body.get("summary").is_none(), "auto must not send a client summary");
}

#[tokio::test]
async fn exchange_is_the_proxy() {
    let s = server().await;
    Mock::given(method("GET"))
        .and(path("/v1/retrieve"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "namespace": "ns",
            "hits": [{"path": "a.md", "score": 1.0, "snippet": "prefers Rust"}],
            "millis": 1
        })))
        .mount(&s)
        .await;
    let client = Client::new("gl_test").with_base_url(s.uri());
    let mut conv = Conversation::create(&client, "c1", opts("gpt-4o", false)).await.unwrap();

    let reply = conv
        .exchange(vec![Message::user("what do I prefer?")], |window| async move {
            // The SDK prepared everything: memory context leads, the user
            // message closes.
            let first = window.first().unwrap();
            assert_eq!(first.role, "system");
            assert!(first.text().contains("prefers Rust"));
            assert_eq!(window.last().unwrap().text(), "what do I prefer?");
            Ok((Message::assistant("you prefer Rust"), None))
        })
        .await
        .unwrap();
    assert_eq!(reply.text(), "you prefer Rust");

    // Both turns stored without the caller appending anything.
    let reqs = s.received_requests().await.unwrap();
    let append = reqs.iter().find(|r| r.url.path().ends_with("/messages")).unwrap();
    let body: Value = append.body_json().unwrap();
    assert_eq!(body["messages"].as_array().unwrap().len(), 2);
}
