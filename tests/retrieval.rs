//! Retrieval, vocabulary and skills against a fake API.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use gitloom::{Client, Error, Mode, RecallOptions, Skill, SkillOptions, Term};

/// Every query string the fake saw, in order.
type Seen = Arc<Mutex<Vec<HashMap<String, String>>>>;

fn record(seen: &Seen, req: &Request) {
    let pairs = req
        .url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    seen.lock().unwrap().push(pairs);
}

async fn retrieving(body: Value) -> (MockServer, Client, Seen) {
    let s = MockServer::start().await;
    let seen: Seen = Default::default();
    let captured = seen.clone();
    Mock::given(method("GET"))
        .and(path("/v1/retrieve"))
        .respond_with(move |req: &Request| {
            record(&captured, req);
            ResponseTemplate::new(200).set_body_json(body.clone())
        })
        .mount(&s)
        .await;
    let client = Client::new("gl_test")
        .with_base_url(s.uri())
        .with_namespace("ns");
    (s, client, seen)
}

fn one_memory() -> Value {
    json!({
        "namespace": "ns",
        "query": "deploys",
        "mode": "raw",
        "memories": [{
            "path": "facts/ops/deploys.md",
            "tier": "facts",
            "topic": "ops",
            "title": "Deploys",
            "content": "Deploys run from main.",
            "score": 0.84,
            "matched": ["lexical", "cue"],
            "sections": ["Process"],
            "tags": ["ops"],
            "scores": {"lexical": 0.7, "cue": 0.6, "coverage": 1.0}
        }],
        "candidates": 4,
        "filtered_out": 3,
        "millis": 12
    })
}

#[tokio::test]
async fn recall_returns_memories_and_sends_every_filter() {
    let (_s, client, seen) = retrieving(one_memory()).await;

    let res = client
        .recall_with(
            "deploys",
            &RecallOptions {
                limit: Some(5),
                tiers: vec!["facts".into(), "rules".into()],
                paths: vec!["facts/ops".into()],
                tags: vec!["ops".into()],
                tags_all: vec!["ops".into(), "prod".into()],
                since: Some("2026-01-01".into()),
                until: Some("2026-09-01".into()),
                min_score: Some(0.3),
                no_context: true,
                detail: Some("full".into()),
                include_expired: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let m = &res.memories[0];
    assert_eq!(m.path, "facts/ops/deploys.md");
    assert_eq!(m.content, "Deploys run from main.");
    assert_eq!(m.matched, ["lexical", "cue"]);
    assert_eq!(m.scores.as_ref().unwrap().coverage, Some(1.0));
    assert_eq!(res.candidates, 4);
    assert_eq!(res.filtered_out, 3);

    let q = &seen.lock().unwrap()[0];
    assert_eq!(q["q"], "deploys");
    assert_eq!(q["namespace"], "ns");
    assert_eq!(q["limit"], "5");
    assert_eq!(q["tiers"], "facts,rules");
    assert_eq!(q["paths"], "facts/ops");
    assert_eq!(q["tags"], "ops");
    assert_eq!(q["tags_all"], "ops,prod");
    assert_eq!(q["since"], "2026-01-01");
    assert_eq!(q["until"], "2026-09-01");
    assert_eq!(q["min_score"], "0.3");
    assert_eq!(q["context"], "0");
    assert_eq!(q["detail"], "full");
    assert_eq!(q["include_expired"], "1");
}

#[tokio::test]
async fn recall_leaves_defaults_off_the_wire() {
    let (_s, client, seen) = retrieving(one_memory()).await;
    client.recall("deploys", None).await.unwrap();

    let q = &seen.lock().unwrap()[0];
    let sent: Vec<&str> = q.keys().map(String::as_str).collect();
    for key in ["mode", "tiers", "paths", "tags", "context", "detail"] {
        assert!(!sent.contains(&key), "a default was sent: {key}");
    }
}

#[tokio::test]
async fn answer_summarizes_and_goes_agentic_on_request() {
    let mut body = one_memory();
    body["answer"] = json!("Deploys run from main.");
    body["model"] = json!("fast");
    let (_s, client, seen) = retrieving(body).await;

    let res = client
        .answer("how do deploys work?", &Default::default())
        .await
        .unwrap();
    assert_eq!(res.answer.as_deref(), Some("Deploys run from main."));

    client
        .answer(
            "how do deploys work?",
            &RecallOptions {
                mode: Mode::Agentic,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let q = seen.lock().unwrap();
    assert_eq!(q[0]["mode"], "summary");
    assert_eq!(q[1]["mode"], "agentic");
}

#[tokio::test]
async fn answer_refuses_to_return_nothing() {
    // The model found nothing to say. An empty string must not reach a caller
    // who would render it as the answer.
    let (_s, client, _) = retrieving(one_memory()).await;
    match client.answer("anything?", &Default::default()).await {
        Err(Error::NoAnswer) => {}
        other => panic!("expected NoAnswer, got {other:?}"),
    }
}

#[tokio::test]
async fn vocab_round_trip() {
    let s = MockServer::start().await;
    let seen: Seen = Default::default();
    let captured = seen.clone();
    Mock::given(method("POST"))
        .and(path("/v1/vocab"))
        .respond_with(
            ResponseTemplate::new(202)
                .set_body_json(json!({"id": "m-1", "namespace": "ns", "status": "accepted"})),
        )
        .mount(&s)
        .await;
    let listed = captured.clone();
    Mock::given(method("GET"))
        .and(path("/v1/vocab"))
        .respond_with(move |req: &Request| {
            record(&listed, req);
            let word = req.url.query_pairs().find(|(k, _)| k == "word").is_some();
            ResponseTemplate::new(200).set_body_json(if word {
                json!({"found": true, "term": {"term": "canary", "aliases": ["baseline rollout"]}})
            } else {
                json!({"terms": [{"term": "canary", "aliases": ["baseline rollout"]}]})
            })
        })
        .mount(&s)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1/vocab"))
        .respond_with(move |req: &Request| {
            record(&captured, req);
            ResponseTemplate::new(202)
                .set_body_json(json!({"id": "m-2", "namespace": "ns", "status": "accepted"}))
        })
        .mount(&s)
        .await;
    let client = Client::new("gl_test")
        .with_base_url(s.uri())
        .with_namespace("ns");

    let accepted = client
        .learn_terms(
            &[Term {
                term: "canary".into(),
                aliases: vec!["baseline rollout".into()],
                ..Default::default()
            }],
            None,
        )
        .await
        .unwrap();
    assert_eq!(accepted.status, "accepted");

    let terms = client.vocabulary(None, None).await.unwrap();
    assert_eq!(terms[0].term, "canary");

    let hit = client.lookup_term("baseline rollout", None).await.unwrap();
    assert_eq!(hit.unwrap().term, "canary");

    client.forget_terms(&["canary"], None).await.unwrap();
    let q = seen.lock().unwrap();
    assert_eq!(q.last().unwrap()["term"], "canary");
}

#[tokio::test]
async fn lookup_of_an_unknown_word_is_not_an_error() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/vocab"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"found": false})))
        .mount(&s)
        .await;
    let client = Client::new("gl_test")
        .with_base_url(s.uri())
        .with_namespace("ns");
    assert!(client.lookup_term("nope", None).await.unwrap().is_none());
}

#[tokio::test]
async fn skills_store_and_find() {
    let s = MockServer::start().await;
    let seen: Seen = Default::default();
    let captured = seen.clone();
    Mock::given(method("POST"))
        .and(path("/v1/skills"))
        .respond_with(
            ResponseTemplate::new(202)
                .set_body_json(json!({"id": "m-3", "namespace": "ns", "status": "accepted",
                    "paths": ["skills/ops/rollback.md"]})),
        )
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/skills"))
        .respond_with(move |req: &Request| {
            record(&captured, req);
            ResponseTemplate::new(200).set_body_json(json!({"skills": [{
                "name": "rollback",
                "topic": "ops",
                "path": "skills/ops/rollback.md",
                "content": "## Revert\nGit revert the deploy commit.",
                "triggers": ["how do I undo a deploy?"],
                "score": 0.77,
                "matched": ["cue"]
            }]}))
        })
        .mount(&s)
        .await;
    let client = Client::new("gl_test")
        .with_base_url(s.uri())
        .with_namespace("ns");

    let accepted = client
        .store_skills(
            &[Skill {
                name: "rollback".into(),
                topic: Some("ops".into()),
                content: Some("## Revert\nGit revert the deploy commit.".into()),
                triggers: vec!["how do I undo a deploy?".into()],
                ..Default::default()
            }],
            None,
        )
        .await
        .unwrap();
    assert_eq!(accepted.paths, ["skills/ops/rollback.md"]);

    let skills = client
        .find_skills(
            "undo a bad deploy",
            &SkillOptions {
                paths: vec!["ops".into()],
                tags: vec!["prod".into()],
                limit: Some(3),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(skills[0].name, "rollback");
    assert!(skills[0].content.as_deref().unwrap().contains("Git revert"));
    assert_eq!(skills[0].matched, ["cue"]);

    let q = &seen.lock().unwrap()[0];
    assert_eq!(q["q"], "undo a bad deploy");
    assert_eq!(q["paths"], "ops");
    assert_eq!(q["tags"], "prod");
    assert_eq!(q["limit"], "3");
}
