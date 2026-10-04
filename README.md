# gitloom

Rust SDK for [GitLoom](https://gitloom.cloud) — conversations that cannot
outgrow their context window, backed by a memory the model can consult.

Provider-agnostic core; the `openai` feature adds conversions for
[`async-openai`](https://crates.io/crates/async-openai).

```toml
[dependencies]
gitloom = { version = "0.4", features = ["openai"] }
tokio = { version = "1", features = ["full"] }
```

## Quickstart

```rust,no_run
use std::time::{Duration, SystemTime};

use gitloom::{Client, NewMemory};

#[tokio::main]
async fn main() -> Result<(), gitloom::Error> {
    let client = Client::new(""); // reads GITLOOM_API_KEY
    client.create_namespace("default").await?;

    client
        .write(
            &[NewMemory {
                path: "facts/places/home.md".into(),
                content: "I live in Pune.".into(),
                tags: vec!["#home".into()],
                occurred_at: Some(SystemTime::now().into()),
                ..Default::default()
            }],
            None,
        )
        .await?;
    tokio::time::sleep(Duration::from_secs(10)).await; // writes land within seconds

    let res = client.recall("where do I live?", None).await?;
    for m in &res.memories {
        println!("{:.2} {} {}", m.score, m.path, m.content);
    }
    Ok(())
}
```

## The loop

```rust,ignore
use gitloom::{Client, Conversation, ConversationOptions, Message, Summarizer};

let client = Client::new("");           // reads GITLOOM_API_KEY
let mut conv = Conversation::create(&client, "chat-42", ConversationOptions {
    model: "gpt-4o".into(),
    namespace: Some(user_id),
    summarize: Some(my_summarizer),     // runs locally, on your model
    ..Default::default()
}).await?;

let user = Message::user("What camera do I own?");
let context = conv.with_context(&user.text()).await?;   // memory, retrieved
let mut window = conv.for_model();                       // fits the model's budget
// ... call your provider with `window` (+ context) ...

conv.append(vec![user, Message::assistant(reply)],
            Some(&gitloom::openai::usage_of(&response))).await?;  // real token counts
```

- The window never overflows: `for_model()` stays inside the model's budget,
  oldest turns evicted first.
- Compaction runs on a cadence (default every 5 exchanges) or when the window
  fills — summarized locally; GitLoom never sees the conversation to compact
  it. Each compaction feeds the evicted turns to memory ingestion.
- Untitled conversations get a title automatically at ingestion.

## Multimodal

```rust,ignore
use gitloom::{Content, Message, Part};

conv.append(vec![Message {
    role: "user".into(),
    content: Content::Parts(vec![
        Part::text_part("what's in this photo?"),
        Part::image_data(b64, "image/png"),  // uploaded transparently, stored by reference
    ]),
    ..Default::default()
}], None).await?;
```

## Branching, edits, rewind

```rust,ignore
conv.rewind(6).await?;                                    // fork after seq 6
conv.edit(4, Message::user("ask differently")).await?;    // fork at same seq
conv.edit_in_place(4, "[redacted]").await?;               // destroy the original (PII)
conv.set_title("Camera shopping").await?;
```

## Recall, filtered and answered

```rust,ignore
client.remember(&[Message::user("I moved to Pune.")], None).await?;

// Ranked memories, no model call. Milliseconds.
let res = client.recall_with("where do I live?", &RecallOptions {
    tiers: vec!["facts".into()],          // facts | incidents | rules | skills
    paths: vec!["facts/places".into()],   // any directories
    tags: vec!["home".into()],
    since: Some("2026-01-01".into()),
    min_score: Some(0.3),
    limit: Some(8),
    ..Default::default()
}).await?;
for m in &res.memories {
    println!("{:.2} {} {:?}\n{}", m.score, m.path, m.matched, m.content);
}

// One text answer from a fast model over that retrieval …
let summary = client.answer("where do I live?", &Default::default()).await?;
// … or let a stronger model search the memory itself with tools.
let agentic = client.answer("which trip had the longest flight?", &RecallOptions {
    mode: Mode::Agentic,
    ..Default::default()
}).await?;
println!("{} {:?}", agentic.answer.unwrap(), agentic.trace);
```

Each entry is one whole memory, not a scattering of its sections, and its
score is calibrated in `[0, 1]` — comparable across queries, so `min_score`
means the same thing every time. `detail: "full"` adds git history with the
last diff, labelled relation snippets and cues.

`answer` meters as a chat rather than a read, and returns `Error::NoAnswer`
rather than an empty string when the model finds nothing to say.

### The lane path

`rank` retrieves on the lane path: lexical, cue, body, graph and time lanes each
search on their own, over the curated memories and the conversation turns, and
the time lane reads dates in the question ("last month", "in May"). `Rank::Fused`
orders what they find by lane score; `Rank::Jev` has a ranking model order it,
and sets `rank_fallback` when it answers in lane order instead.

```rust,ignore
use gitloom::{Rank, ReaderModel, RecallOptions};

let res = client.recall_with("when did I stake the tomatoes", &RecallOptions {
    rank: Some(Rank::Fused),
    max_chars: Some(8000),
    ..Default::default()
}).await?;
for m in &res.memories {
    println!("{:?} {:?} {} {}", m.store, m.said, m.excerpted, m.content);
}

let res = client.answer("what did I plant after the storm", &RecallOptions {
    rank: Some(Rank::Jev),
    model: Some(ReaderModel::Sonnet),
    ..Default::default()
}).await?;
```

Each memory then says which `store` it came from (`memory`, or a word-for-word
conversation `turn`) and the days it was `said`. `max_chars` caps the memory
content returned: a memory that does not fit is cut to its opening sentence and
the sentences matching the question, and marked `excerpted`. `model` picks the
model that reads the memories in `Mode::Summary` or `Mode::Agentic`.

## Tags and times

```rust,ignore
use std::time::SystemTime;
use gitloom::{NewMemory, RememberOptions};

// A conversation: the tags go on every memory drawn from it.
client.remember_with(&turns, &RememberOptions {
    tags: vec!["#launch".into(), "team:core".into()],
    occurred_at: Some("2026-09-12T18:30:00".into()),   // when it happened,
    timezone: Some("Asia/Kolkata".into()),             // read in this zone
    ..Default::default()
}).await?;

// A memory already formed.
client.write(&[NewMemory {
    path: "incidents/ops/outage.md".into(),
    content: "The API was down for 40 minutes.".into(),
    tags: vec!["#outage".into()],
    occurred_at: Some(SystemTime::now().into()),       // or 1_757_700_000.into(), or "2026-09-12".into()
    ..Default::default()
}], None).await?;
```

`occurred_at` is when the thing happened, not when it was written. It takes a
`SystemTime` (sent as epoch seconds), an integer epoch, or a string sent as-is:
RFC 3339 with an offset, a date `YYYY-MM-DD` meaning that calendar day, or a
datetime without an offset, read in `timezone`. `date` still works and is
deprecated.

Tags are trimmed and lowercased: letters, digits, spaces and `- _ . : / # @`, at
most 32 of 64 characters each. A refused one comes back as `Error::Api` with
code `invalid_tag` and a message naming the field, e.g. `memories[1].tags[0]`.

Every recalled memory carries `tags` (yours first), `user_tags` (yours alone),
and `created_at`, `updated_at`, `occurred_at` and `expires_at` as
`Option<SystemTime>`. `occurred_source` says how `occurred_at` is known (`user`,
`extracted`, `said` or `written`), and `occurred_precision` is `day` when only
the date is known (held as noon UTC on it) or `instant`. The `created` and
`updated` strings are deprecated.

## Recall by filter alone

```rust,ignore
use gitloom::{RecallOptions, TimeField};

// No query: every memory tagged #launch that happened in September, newest first.
let res = client.recall_with("", &RecallOptions {
    tags: vec!["#launch".into()],
    since: Some("2026-09-01".into()),
    until: Some("2026-09-30".into()),          // a date-only until takes in that whole day
    time_field: Some(TimeField::Occurred),     // Occurred | Created | Updated (the default)
    tz: Some("Asia/Kolkata".into()),
    ..Default::default()
}).await?;
```

With an empty query, `recall_with` lists every memory the filters match, newest
first by `time_field`, each scored 1. It needs at least one of `tags`,
`tags_all`, `since`, `until`, `tiers` or `paths` — without one it returns
`Error::Usage` before sending anything — and takes `Mode::Raw` with no `rank`.
With a query, the same filters narrow the search. Values are URL-encoded for
you, so a `#` in a tag arrives intact.

## Memories by path

```rust,ignore
use gitloom::{NewMemory, TopicsOptions, TreeOptions};

client.topics(&TopicsOptions { like: Some("databas".into()), ..Default::default() }).await?;
client.write(&[NewMemory {
    path: "facts/people/maya.md".into(),       // the directory is the topic
    content: "Maya rides a bicycle to work.".into(),
    cues: vec!["how does Maya get around".into()],
    related: vec!["spouse: facts/people/sam.md".into()],
    ..Default::default()
}], None).await?;

let m = client.get("facts/people/maya.md", None).await?;
let toc = client.tree(&TreeOptions { path: Some("facts".into()), depth: Some(3), ..Default::default() }).await?;
let graph = client.graph(&Default::default()).await?;
client.forget(&["facts/people/maya.md"], None).await?;
```

`write` stores memories as given, where `remember` has a model decide what a
conversation holds; send them in batches, since one call is one commit round.
`get` reads a file or a `file.md#section`, `tree` is the table of contents,
`topics` the directories with their memory counts — check it before inventing
a topic — and `graph` the relationships between memories. `write` and `forget`
are asynchronous, like `remember`.

## Vocabulary and skills

```rust,ignore
// Teach abbreviations and domain terms. A recall for "k8s" then also finds
// memories written "kubernetes", and the definition comes back as `defined`.
client.learn_terms(&[Term {
    term: "kubernetes".into(),
    aliases: vec!["k8s".into(), "kube".into()],
    definition: Some("Container orchestration.".into()),
    ..Default::default()
}], None).await?;
client.lookup_term("k8s", None).await?;      // → Some(Term { term: "kubernetes", .. })
client.vocabulary(Some("kube"), None).await?;
client.forget_terms(&["kubernetes"], None).await?;

// Store how things are done; find the skill that fits a task.
client.store_skills(&[Skill {
    name: "Deploy to production".into(),
    topic: Some("ops".into()),
    description: Some("Ship a release.".into()),
    content: Some("## Steps\n1. Tag the release.\n2. `make deploy ENV=prod`".into()),
    triggers: vec!["how do I ship a release".into(), "deploy to prod".into()],
    ..Default::default()
}], None).await?;
let skills = client.find_skills("release the new build", &Default::default()).await?;
```

Skills are memories under the `skills/` tier, so a recall with
`tiers: vec!["skills".into()]` reaches them too.

## Docs

https://docs.gitloom.cloud/documentation/rust
