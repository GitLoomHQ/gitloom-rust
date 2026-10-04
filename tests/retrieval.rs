//! Retrieval, vocabulary and skills against a fake API.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use gitloom::{
    Client, Error, Mode, Rank, ReaderModel, RecallOptions, Skill, SkillOptions, Term, TimeField,
    Timestamp,
};

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
    for key in [
        "mode",
        "tiers",
        "paths",
        "tags",
        "context",
        "detail",
        "since",
        "until",
        "time_field",
        "tz",
    ] {
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
async fn recall_sends_the_lane_path_and_reads_what_it_adds() {
    let (_s, client, seen) = retrieving(json!({
        "namespace": "ns",
        "query": "x",
        "mode": "summary",
        "rank": "jev",
        "rank_fallback": true,
        "memories": [{
            "path": "turns/conv-1/main/000001-user-aa.md",
            "tier": "facts",
            "content": "user: I staked the tomatoes …",
            "score": 0.8,
            "matched": ["lexical", "time"],
            "store": "turn",
            "said": ["2026-05-21"],
            "excerpted": true
        }],
        "candidates": 9,
        "filtered_out": 0,
        "millis": 40,
        "timings": {
            "lexical_ms": 0, "vector_ms": 0, "graph_ms": 0,
            "embed_ms": 20, "lanes_ms": 8, "rank_ms": 300,
            "lane": [{"lane": "time", "store": "turn", "ms": 2, "n": 1}]
        }
    }))
    .await;

    let res = client
        .recall_with(
            "x",
            &RecallOptions {
                mode: Mode::Summary,
                rank: Some(Rank::Jev),
                max_chars: Some(12000),
                model: Some(ReaderModel::Sonnet),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let q = &seen.lock().unwrap()[0];
    assert_eq!(q["rank"], "jev");
    assert_eq!(q["max_chars"], "12000");
    assert_eq!(q["model"], "sonnet");

    assert_eq!(res.rank.as_deref(), Some("jev"));
    assert!(res.rank_fallback);
    let m = &res.memories[0];
    assert_eq!(m.store.as_deref(), Some("turn"));
    assert_eq!(m.said, ["2026-05-21"]);
    assert!(m.excerpted);
    assert_eq!(m.matched, ["lexical", "time"]);
    assert_eq!(res.timings.embed_ms, Some(20));
    assert_eq!(res.timings.lanes_ms, Some(8));
    assert_eq!(res.timings.rank_ms, Some(300));
    let lane = &res.timings.lane[0];
    assert_eq!((lane.lane.as_str(), lane.store.as_str()), ("time", "turn"));
    assert_eq!((lane.ms, lane.n, lane.err.as_deref()), (2, 1, None));
}

#[tokio::test]
async fn lane_path_stays_off_the_wire_unless_asked_for() {
    let (_s, client, seen) = retrieving(one_memory()).await;
    let res = client.recall("deploys", None).await.unwrap();
    client
        .recall_with(
            "deploys",
            &RecallOptions {
                max_chars: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    for q in seen.lock().unwrap().iter() {
        let mut sent: Vec<&str> = q.keys().map(String::as_str).collect();
        sent.sort();
        assert_eq!(sent, ["namespace", "q"]);
    }

    assert!(res.rank.is_none());
    assert!(!res.rank_fallback);
    let m = &res.memories[0];
    assert!(m.store.is_none());
    assert!(m.said.is_empty());
    assert!(!m.excerpted);
    assert!(res.timings.embed_ms.is_none());
    assert!(res.timings.lane.is_empty());
}

#[tokio::test]
async fn answer_passes_the_lane_path_through() {
    let mut body = one_memory();
    body["answer"] = json!("A.");
    let (_s, client, seen) = retrieving(body).await;

    client
        .answer(
            "x",
            &RecallOptions {
                rank: Some(Rank::Fused),
                max_chars: Some(8000),
                model: Some(ReaderModel::Haiku),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let q = &seen.lock().unwrap()[0];
    assert_eq!(q["mode"], "summary");
    assert_eq!(q["rank"], "fused");
    assert_eq!(q["max_chars"], "8000");
    assert_eq!(q["model"], "haiku");
}

#[tokio::test]
async fn recall_sends_tags_and_times() {
    let (s, client, seen) = retrieving(one_memory()).await;
    client
        .recall_with(
            "launch",
            &RecallOptions {
                tags: vec!["#launch".into(), "team:core".into()],
                tags_all: vec!["q3 plan".into()],
                since: Some((UNIX_EPOCH + Duration::from_millis(1_760_000_000_900)).into()),
                until: Some("2026-09-30".into()),
                time_field: Some(TimeField::Occurred),
                tz: Some("Asia/Kolkata".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // A raw # would end the query string at the tag.
    let raw = s.received_requests().await.unwrap()[0]
        .url
        .query()
        .unwrap()
        .to_string();
    assert!(raw.contains("tags=%23launch%2Cteam%3Acore"), "{raw}");
    assert!(!raw.contains('#'));

    let q = &seen.lock().unwrap()[0];
    assert_eq!(q["tags"], "#launch,team:core");
    assert_eq!(q["tags_all"], "q3 plan");
    assert_eq!(q["since"], "1760000000");
    assert_eq!(q["until"], "2026-09-30");
    assert_eq!(q["time_field"], "occurred");
    assert_eq!(q["tz"], "Asia/Kolkata");
}

#[tokio::test]
async fn every_time_field_and_epoch_go_out_as_the_server_spells_them() {
    let (_s, client, seen) = retrieving(one_memory()).await;
    for field in [TimeField::Occurred, TimeField::Created, TimeField::Updated] {
        client
            .recall_with(
                "x",
                &RecallOptions {
                    since: Some(1_700_000_000.into()),
                    time_field: Some(field),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }
    let q = seen.lock().unwrap();
    let fields: Vec<&str> = q.iter().map(|q| q["time_field"].as_str()).collect();
    assert_eq!(fields, ["occurred", "created", "updated"]);
    assert_eq!(q[0]["since"], "1700000000");
}

#[test]
fn a_system_time_in_the_servers_epoch_range_goes_as_seconds() {
    let t = UNIX_EPOCH + Duration::from_millis(1_760_000_000_999);
    assert_eq!(Timestamp::from(t), Timestamp::Epoch(1_760_000_000));
    let first = UNIX_EPOCH + Duration::from_secs(100_000_000);
    assert_eq!(Timestamp::from(first), Timestamp::Epoch(100_000_000));
}

#[test]
fn a_system_time_outside_it_goes_as_rfc3339_utc() {
    let text = |t| match Timestamp::from(t) {
        Timestamp::Text(s) => s,
        other => panic!("expected text, got {other:?}"),
    };
    // 1965-04-02T10:00:00Z
    assert_eq!(
        text(UNIX_EPOCH - Duration::from_secs(149_868_000)),
        "1965-04-02T10:00:00Z"
    );
    // Floored, not truncated toward the epoch.
    assert_eq!(
        text(UNIX_EPOCH - Duration::from_millis(500)),
        "1969-12-31T23:59:59Z"
    );
    assert_eq!(
        text(UNIX_EPOCH - Duration::from_millis(1500)),
        "1969-12-31T23:59:58Z"
    );
    assert_eq!(text(UNIX_EPOCH), "1970-01-01T00:00:00Z");
    assert_eq!(
        text(UNIX_EPOCH + Duration::from_millis(99_999_999_900)),
        "1973-03-03T09:46:39Z"
    );
    // 1964-02-29T12:00:00Z
    assert_eq!(
        text(UNIX_EPOCH - Duration::from_secs(184_248_000)),
        "1964-02-29T12:00:00Z"
    );
    assert_eq!(
        text(UNIX_EPOCH + Duration::from_secs(100_000_000_000)),
        "5138-11-16T09:46:40Z"
    );
}

#[test]
fn an_explicit_epoch_is_sent_as_given() {
    assert_eq!(Timestamp::from(-5i64), Timestamp::Epoch(-5));
    assert_eq!(Timestamp::from(0i64), Timestamp::Epoch(0));
}

#[tokio::test]
async fn recall_without_a_query_lists_what_the_filters_match() {
    let mut body = one_memory();
    body["query"] = json!("");
    body["memories"][0]["score"] = json!(1);
    let (_s, client, seen) = retrieving(body).await;

    let res = client
        .recall_with(
            "",
            &RecallOptions {
                tags: vec!["#launch".into()],
                time_field: Some(TimeField::Occurred),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(res.memories[0].score, 1.0);

    for filter in [
        RecallOptions {
            tags_all: vec!["ops".into()],
            ..Default::default()
        },
        RecallOptions {
            since: Some("2026-01-01".into()),
            ..Default::default()
        },
        RecallOptions {
            until: Some(1_760_000_000.into()),
            ..Default::default()
        },
        RecallOptions {
            tiers: vec!["facts".into()],
            ..Default::default()
        },
        RecallOptions {
            paths: vec!["facts/ops".into()],
            ..Default::default()
        },
    ] {
        client.recall_with("  ", &filter).await.unwrap();
    }

    let q = seen.lock().unwrap();
    assert_eq!(q.len(), 6);
    for q in q.iter() {
        assert!(!q.contains_key("q"), "a listing sent q: {q:?}");
    }
    assert_eq!(q[0]["tags"], "#launch");
    assert_eq!(q[0]["time_field"], "occurred");
}

#[tokio::test]
async fn recall_with_neither_query_nor_filter_never_reaches_the_server() {
    let (s, client, _) = retrieving(one_memory()).await;
    for opts in [
        RecallOptions::default(),
        // These shape a listing but do not say what to list.
        RecallOptions {
            time_field: Some(TimeField::Created),
            tz: Some("Asia/Kolkata".into()),
            limit: Some(5),
            ..Default::default()
        },
    ] {
        match client.recall_with("", &opts).await {
            Err(Error::Usage(msg)) => assert!(msg.contains("filter"), "{msg}"),
            other => panic!("expected a usage error, got {other:?}"),
        }
    }
    match client.recall("", None).await {
        Err(Error::Usage(_)) => {}
        other => panic!("expected a usage error, got {other:?}"),
    }
    assert!(s.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
#[allow(deprecated)]
async fn memories_carry_their_tags_and_times() {
    let (_s, client, _) = retrieving(json!({
        "namespace": "ns",
        "query": "launch",
        "memories": [{
            "path": "facts/launch.md",
            "content": "We launch on the 12th.",
            "score": 0.9,
            "tags": ["#launch", "planning"],
            "user_tags": ["#launch"],
            "created": "2026-09-01T10:00:00Z",
            "updated": "2026-09-02T10:00:00Z",
            "created_at": 1_788_256_800,
            "updated_at": 1_788_343_200,
            "occurred_at": 1_791_806_400,
            "occurred_source": "user",
            "occurred_precision": "day",
            "expires_at": 1_794_000_000
        }, {
            "path": "facts/old.md",
            "content": "Old.",
            "score": 0.5
        }]
    }))
    .await;

    let res = client.recall("launch", None).await.unwrap();
    let m = &res.memories[0];
    assert_eq!(m.tags, ["#launch", "planning"]);
    assert_eq!(m.user_tags, ["#launch"]);
    let at = |s: u64| Some(UNIX_EPOCH + Duration::from_secs(s));
    assert_eq!(m.created_at, at(1_788_256_800));
    assert_eq!(m.updated_at, at(1_788_343_200));
    assert_eq!(m.occurred_at, at(1_791_806_400));
    assert_eq!(m.expires_at, at(1_794_000_000));
    assert_eq!(m.occurred_source.as_deref(), Some("user"));
    assert_eq!(m.occurred_precision.as_deref(), Some("day"));
    assert_eq!(m.created.as_deref(), Some("2026-09-01T10:00:00Z"));
    assert_eq!(m.updated.as_deref(), Some("2026-09-02T10:00:00Z"));

    let bare = &res.memories[1];
    assert!(bare.user_tags.is_empty());
    assert!(bare.created_at.is_none() && bare.occurred_at.is_none() && bare.expires_at.is_none());
    assert!(bare.occurred_source.is_none() && bare.occurred_precision.is_none());
}

#[tokio::test]
async fn a_refused_filter_comes_back_with_its_code() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/retrieve"))
        .respond_with(ResponseTemplate::new(400).set_body_json(
            json!({"error": {"code": "invalid_date", "message": "since is after until"}}),
        ))
        .mount(&s)
        .await;
    let client = Client::new("gl_test").with_base_url(s.uri());
    let err = client
        .recall_with(
            "x",
            &RecallOptions {
                since: Some("2026-09-01".into()),
                until: Some("2026-01-01".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    match err {
        Error::Api {
            status,
            code,
            message,
        } => {
            assert_eq!((status, code.as_str()), (400, "invalid_date"));
            assert_eq!(message, "since is after until");
        }
        other => panic!("expected an API error, got {other:?}"),
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
