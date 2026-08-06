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

## Memory, directly

```rust,ignore
client.remember(&[Message::user("I moved to Pune.")], None).await?;
let res = client.recall("where do I live?", None).await?;
for hit in &res.hits {
    println!("{} {:?} {:?}", hit.snippet, hit.scores, hit.provenance);
}
```

Every hit carries its evidence — per-arm scores, git history with the last
diff, labelled relation snippets — the same shape every GitLoom surface
returns.

## Docs

https://docs.gitloom.cloud/documentation/rust
