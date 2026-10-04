//! Direct writes, reads by path and navigation against a fake API.

use std::time::{Duration, UNIX_EPOCH};

use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use gitloom::{
    Client, Error, GraphOptions, Message, NewMemory, RememberOptions, TopicsOptions, TreeOptions,
};

async fn replying(verb: &str, route: &str, body: Value) -> (MockServer, Client) {
    let s = MockServer::start().await;
    Mock::given(method(verb))
        .and(path(route))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&s)
        .await;
    let client = Client::new("gl_test")
        .with_base_url(s.uri())
        .with_namespace("ns");
    (s, client)
}

fn accepted() -> Value {
    json!({"id": "m-1", "namespace": "ns", "status": "accepted"})
}

async fn only_request(s: &MockServer) -> Request {
    let mut all = s.received_requests().await.unwrap();
    assert_eq!(all.len(), 1, "expected exactly one request");
    all.remove(0)
}

fn query(req: &Request) -> std::collections::HashMap<String, String> {
    req.url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

#[tokio::test]
#[allow(deprecated)]
async fn write_sends_memories_not_messages() {
    let (s, client) = replying("POST", "/v1/memories", accepted()).await;
    client
        .write(
            &[
                NewMemory {
                    path: "facts/people/maya.md".into(),
                    content: "Maya rides a bicycle.".into(),
                    tags: vec!["people".into(), "#commute".into()],
                    occurred_at: Some(
                        (UNIX_EPOCH + Duration::from_millis(1_752_926_400_750)).into(),
                    ),
                    confidence: Some(0.8),
                    cues: vec!["how does Maya get around".into()],
                    related: vec!["spouse: facts/people/sam.md".into()],
                    ..Default::default()
                },
                NewMemory {
                    path: "incidents/ops/outage.md".into(),
                    content: "The API was down.".into(),
                    occurred_at: Some("2026-07-19T09:30:00".into()),
                    ttl: Some("30d".into()),
                    supersedes: Some("incidents/ops/degraded.md".into()),
                    ..Default::default()
                },
                NewMemory {
                    path: "facts/old.md".into(),
                    content: "Backfilled.".into(),
                    date: Some("2025-03-01".into()),
                    ..Default::default()
                },
            ],
            None,
        )
        .await
        .unwrap();

    let req = only_request(&s).await;
    let body: Value = req.body_json().unwrap();
    // The endpoint dispatches on which key is present: memories are stored as
    // given, messages are handed to a model.
    assert!(body.get("messages").is_none());
    assert_eq!(body["namespace"], "ns");

    let m = &body["memories"];
    assert_eq!(
        m[0],
        json!({
            "path": "facts/people/maya.md",
            "content": "Maya rides a bicycle.",
            "tags": ["people", "#commute"],
            "occurred_at": 1_752_926_400,
            "confidence": 0.8,
            "cues": ["how does Maya get around"],
            "related": ["spouse: facts/people/sam.md"]
        })
    );
    assert_eq!(m[1]["occurred_at"], "2026-07-19T09:30:00");
    assert_eq!(m[1]["ttl"], "30d");
    assert_eq!(m[1]["supersedes"], "incidents/ops/degraded.md");
    assert_eq!(
        m[2],
        json!({"path": "facts/old.md", "content": "Backfilled.", "date": "2025-03-01"})
    );
}

#[tokio::test]
async fn write_refuses_a_bad_path_before_sending() {
    let (s, client) = replying("POST", "/v1/memories", accepted()).await;
    let res = client
        .write(
            &[
                NewMemory {
                    path: "facts/ok.md".into(),
                    content: "x".into(),
                    ..Default::default()
                },
                NewMemory {
                    path: "facts/not-markdown".into(),
                    content: "y".into(),
                    ..Default::default()
                },
            ],
            None,
        )
        .await;
    match res {
        Err(Error::Usage(msg)) => assert!(msg.contains("memory 1"), "{msg}"),
        other => panic!("expected a usage error, got {other:?}"),
    }
    assert!(s.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn writing_or_forgetting_nothing_sends_nothing() {
    let s = MockServer::start().await;
    let client = Client::new("gl_test").with_base_url(s.uri());
    client.write(&[], None).await.unwrap();
    client.forget(&[], None).await.unwrap();
    assert!(s.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
#[allow(deprecated)]
async fn remember_with_tags_the_conversation_and_dates_it() {
    let (s, client) = replying("POST", "/v1/memories", accepted()).await;
    client
        .remember_with(
            &[Message::user("We ship on Friday.")],
            &RememberOptions {
                namespace: Some("team".into()),
                session_id: Some("standup-12".into()),
                tags: vec!["#launch".into(), "standup".into()],
                occurred_at: Some((UNIX_EPOCH + Duration::from_secs(1_791_806_400)).into()),
                timezone: Some("Asia/Kolkata".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    client
        .remember_with(
            &[Message::user("Older.")],
            &RememberOptions {
                date: Some("2025-03-01".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let reqs = s.received_requests().await.unwrap();
    let body: Value = reqs[0].body_json().unwrap();
    assert_eq!(
        body,
        json!({
            "namespace": "team",
            "session_id": "standup-12",
            "messages": [{"role": "user", "content": "We ship on Friday."}],
            "tags": ["#launch", "standup"],
            "occurred_at": 1_791_806_400,
            "timezone": "Asia/Kolkata"
        })
    );
    let body: Value = reqs[1].body_json().unwrap();
    assert_eq!(body["date"], "2025-03-01");
    assert_eq!(body["namespace"], "ns");
}

#[tokio::test]
async fn remember_sends_only_the_conversation() {
    let (s, client) = replying("POST", "/v1/memories", accepted()).await;
    client
        .remember(&[Message::user("I moved to Pune.")], None)
        .await
        .unwrap();
    let body: Value = only_request(&s).await.body_json().unwrap();
    let mut keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(keys, ["messages", "namespace"]);
}

#[tokio::test]
#[allow(deprecated)]
async fn get_reads_tags_and_times() {
    let (s, client) = replying(
        "GET",
        "/v1/memories",
        json!({
            "confidence": 0, "content": "…", "created": "2026-10-04T13:33:23Z",
            "created_at": 1791120803, "kind": "file", "millis": 0, "namespace": "x",
            "occurred_at": 1772712000, "occurred_precision": "day", "occurred_source": "user",
            "path": "facts/test/a.md", "tags": ["home", "lease"], "tier": "facts", "title": "",
            "updated": "2026-10-04T13:33:23Z", "updated_at": 1791120803,
            "user_tags": ["home", "lease"]
        }),
    )
    .await;
    let m = client.get("facts/test/a.md", Some("x")).await.unwrap();
    let at = |s: u64| Some(UNIX_EPOCH + Duration::from_secs(s));
    assert_eq!(m.path, "facts/test/a.md");
    assert_eq!(m.kind.as_deref(), Some("file"));
    assert_eq!(m.tags, ["home", "lease"]);
    assert_eq!(m.user_tags, ["home", "lease"]);
    assert_eq!(m.created_at, at(1791120803));
    assert_eq!(m.updated_at, at(1791120803));
    assert_eq!(m.occurred_at, at(1772712000));
    assert!(m.expires_at.is_none());
    assert_eq!(m.occurred_source.as_deref(), Some("user"));
    assert_eq!(m.occurred_precision.as_deref(), Some("day"));
    assert_eq!(m.created.as_deref(), Some("2026-10-04T13:33:23Z"));
    assert_eq!(m.updated.as_deref(), Some("2026-10-04T13:33:23Z"));

    let q = query(&only_request(&s).await);
    assert_eq!(q["path"], "facts/test/a.md");
    assert_eq!(q["namespace"], "x");
}

#[tokio::test]
async fn get_reads_an_untagged_memory() {
    let (s, client) = replying(
        "GET",
        "/v1/memories",
        json!({
            "namespace": "ns", "path": "facts/people/maya.md#commute", "kind": "section",
            "content": "Maya rides a bicycle.", "tags": null, "user_tags": null,
            "confidence": 0.8, "created_at": 1791120803, "updated_at": 1791120803
        }),
    )
    .await;
    let m = client
        .get("facts/people/maya.md#commute", None)
        .await
        .unwrap();
    assert_eq!(m.content, "Maya rides a bicycle.");
    assert_eq!(m.confidence, Some(0.8));
    assert!(m.tags.is_empty() && m.user_tags.is_empty());
    assert!(m.cues.is_empty() && m.related.is_empty());
    assert!(m.occurred_at.is_none() && m.occurred_source.is_none());

    let req = only_request(&s).await;
    assert_eq!(query(&req)["path"], "facts/people/maya.md#commute");
    assert!(req.url.query().unwrap().contains("%23commute"));
}

#[tokio::test]
async fn forget_puts_paths_in_the_query() {
    let s = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/v1/memories"))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({
            "id": "m-2", "namespace": "other", "status": "accepted",
            "paths": ["facts/a.md", "facts/b.md"]
        })))
        .mount(&s)
        .await;
    let client = Client::new("gl_test").with_base_url(s.uri());
    client
        .forget(&["facts/a.md", "facts/b.md"], Some("other"))
        .await
        .unwrap();

    // Several HTTP clients decline to send a body on DELETE.
    let req = only_request(&s).await;
    assert!(req.body.is_empty());
    let q = query(&req);
    assert_eq!(q["path"], "facts/a.md,facts/b.md");
    assert_eq!(q["namespace"], "other");
}

#[tokio::test]
async fn tree_carries_its_root_and_depth() {
    let (s, client) = replying(
        "GET",
        "/v1/tree",
        json!({
            "namespace": "ns",
            "depth": 3,
            "tree": {
                "path": "facts",
                "kind": "dir",
                "children": [{
                    "path": "facts/people",
                    "title": "People",
                    "summary": "Who is who.",
                    "children": [{"path": "facts/people/maya.md", "kind": "file"}]
                }]
            },
            "millis": 4
        }),
    )
    .await;
    let res = client
        .tree(&TreeOptions {
            path: Some("facts".into()),
            depth: Some(3),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(res.depth, 3);
    let people = &res.tree.children[0];
    assert_eq!(people.summary.as_deref(), Some("Who is who."));
    assert_eq!(people.children[0].path, "facts/people/maya.md");
    assert!(people.children[0].children.is_empty());

    let q = query(&only_request(&s).await);
    assert_eq!(q["path"], "facts");
    assert_eq!(q["depth"], "3");
    assert_eq!(q["namespace"], "ns");
}

#[tokio::test]
async fn tree_and_topics_leave_unset_options_off_the_wire() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/tree"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"tree": {"path": ""}})))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/topics"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"topics": []})))
        .mount(&s)
        .await;
    let client = Client::new("gl_test")
        .with_base_url(s.uri())
        .with_namespace("ns");
    client.tree(&Default::default()).await.unwrap();
    client.topics(&Default::default()).await.unwrap();
    for req in s.received_requests().await.unwrap() {
        assert_eq!(req.url.query(), Some("namespace=ns"));
    }
}

#[tokio::test]
async fn topics_carry_their_filters() {
    let (s, client) = replying(
        "GET",
        "/v1/topics",
        json!({
            "namespace": "ns",
            "topics": [{
                "path": "facts/databases", "name": "databases", "tier": "facts",
                "parent": "facts", "depth": 1, "memories": 7
            }],
            "millis": 2
        }),
    )
    .await;
    let res = client
        .topics(&TopicsOptions {
            tier: Some("facts".into()),
            prefix: Some("facts".into()),
            like: Some("databas".into()),
            max_depth: Some(2),
            min_files: Some(2),
            limit: Some(50),
            ..Default::default()
        })
        .await
        .unwrap();
    let t = &res.topics[0];
    assert_eq!((t.name.as_str(), t.memories, t.depth), ("databases", 7, 1));

    let q = query(&only_request(&s).await);
    for (k, v) in [
        ("tier", "facts"),
        ("prefix", "facts"),
        ("like", "databas"),
        ("max_depth", "2"),
        ("min_files", "2"),
        ("limit", "50"),
    ] {
        assert_eq!(q[k], v, "{k}");
    }
}

#[tokio::test]
async fn graph_decodes_nodes_and_edges() {
    let (s, client) = replying(
        "GET",
        "/v1/graph",
        json!({
            "namespace": "ns",
            "nodes": [
                {"path": "facts/a.md", "tier": "facts", "kind": "file", "title": "A", "hash": "abc1234"},
                {"path": "facts/b.md", "tier": "facts", "kind": "file"}
            ],
            "edges": [{"src": "facts/a.md", "dst": "facts/b.md", "label": "spouse", "origin": "frontmatter"}],
            "truncated": true,
            "millis": 9,
            "index": {"files": 2, "sections": 0, "cues": 0, "embedded": 0}
        }),
    )
    .await;
    let g = client
        .graph(&GraphOptions {
            limit: Some(100),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(g.nodes.len(), 2);
    assert_eq!(g.nodes[0].title.as_deref(), Some("A"));
    assert_eq!(g.edges[0].label.as_deref(), Some("spouse"));
    assert!(g.truncated);

    let q = query(&only_request(&s).await);
    assert_eq!(q["limit"], "100");
    assert_eq!(q["namespace"], "ns");
}

#[tokio::test]
async fn an_empty_graph_decodes() {
    // The server sends null, not [], for a namespace with nothing in it.
    let (_s, client) = replying(
        "GET",
        "/v1/graph",
        json!({"namespace": "ns", "nodes": null, "edges": null, "truncated": false, "millis": 1}),
    )
    .await;
    let g = client.graph(&Default::default()).await.unwrap();
    assert!(g.nodes.is_empty() && g.edges.is_empty());
}

#[tokio::test]
async fn a_refused_tag_comes_back_with_its_code() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/memories"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": {
            "code": "invalid_tag",
            "message": "memories[0].tags[0] \"a|b\" may hold only letters, digits, spaces and - _ . : / # @"
        }})))
        .mount(&s)
        .await;
    let client = Client::new("gl_test").with_base_url(s.uri());
    let err = client
        .write(
            &[NewMemory {
                path: "facts/a.md".into(),
                content: "x".into(),
                tags: vec!["a|b".into()],
                ..Default::default()
            }],
            None,
        )
        .await
        .unwrap_err();
    match err {
        Error::Api {
            status,
            code,
            message,
        } => {
            assert_eq!((status, code.as_str()), (400, "invalid_tag"));
            assert!(message.starts_with("memories[0].tags[0]"));
        }
        other => panic!("expected an API error, got {other:?}"),
    }
}
