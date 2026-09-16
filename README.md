# gitloom

Rust SDK for [GitLoom](https://gitloom.cloud) — conversations that cannot
outgrow their context window, backed by a memory the model can consult.

Provider-agnostic core; the `openai` feature adds conversions for
[`async-openai`](https://crates.io/crates/async-openai).

```toml
[dependencies]
gitloom = { version = "0.1", features = ["openai"] }
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
