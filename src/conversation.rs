//! A stored chat with a rolling context window.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::client::{Client, Error};
use crate::tokens::{context_limit, total_tokens};
use crate::types::{BranchInfo, Content, Message, Usage};

/// Turns evicted messages into one summary. Runs locally — GitLoom never sees
/// the conversation to compact it.
pub type Summarizer = Box<dyn Fn(&[Message]) -> Result<String, Error> + Send + Sync>;

/// How memory is consulted as the conversation runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MemoryMode {
    /// `with_context()` retrieves for every user message.
    #[default]
    Query,
    /// The developer wires memory tools; the model decides.
    Tools,
    /// Store and compact only.
    Off,
}

/// Configuration for a managed conversation.
#[derive(Default)]
pub struct ConversationOptions {
    /// Model whose window bounds the conversation, e.g. "gpt-4o".
    pub model: String,
    /// Overrides the model's inferred window.
    pub max_tokens: Option<u32>,
    /// Tokens held back for the reply.
    pub reserve_for_reply: u32,
    /// Fill fraction that triggers compaction. Default 0.85.
    pub compact_at: Option<f64>,
    /// Compact after this many exchanges regardless of tokens. Default 5;
    /// Some(0) disables the cadence. Compaction is also the memory trigger.
    pub compact_every: Option<u32>,
    /// Produces compaction summaries. Compaction is refused without one.
    pub summarize: Option<Summarizer>,
    /// Where memories land and are recalled from.
    pub namespace: Option<String>,
    /// Memory mode. Default Query.
    pub memory: MemoryMode,
    /// Title at creation; left empty, ingestion generates one.
    pub title: Option<String>,
}

/// A stored conversation. Obtain from [`Conversation::create`] or
/// [`Conversation::load`].
pub struct Conversation {
    pub id: String,
    pub branch: String,
    pub title: String,

    client: Client,
    opts: ConversationOptions,
    history: Vec<Message>,
    summary: String,
    next_seq: i64,
    first_live_seq: i64,
    exchanges: u32,
    reported_tokens: u32,
}

#[derive(Deserialize)]
struct LoadResponse {
    branch: String,
    #[serde(default)]
    title: String,
    next_seq: i64,
    #[serde(default)]
    messages: Vec<WireMessage>,
    compaction: Option<CompactionInfo>,
}

#[derive(Deserialize)]
struct WireMessage {
    seq: i64,
    #[serde(flatten)]
    message: Message,
}

#[derive(Deserialize)]
struct CompactionInfo {
    summary: String,
}

impl Conversation {
    /// Create (idempotently) a stored conversation.
    pub async fn create(client: &Client, id: &str, opts: ConversationOptions) -> Result<Self, Error> {
        let ns = opts.namespace.clone().unwrap_or_else(|| client.namespace.clone());
        let mut body = json!({"id": id, "namespace": ns});
        if let Some(t) = &opts.title {
            body["title"] = json!(t);
        }
        if !opts.model.is_empty() {
            body["model"] = json!(opts.model);
        }
        #[derive(Deserialize)]
        struct Created {
            branch: String,
            next_seq: i64,
        }
        let res: Created = client
            .request(reqwest::Method::POST, "/v1/conversations", Some(body))
            .await?;
        Ok(Self {
            id: id.into(),
            branch: res.branch,
            title: opts.title.clone().unwrap_or_default(),
            client: client.clone(),
            opts,
            history: Vec::new(),
            summary: String::new(),
            next_seq: res.next_seq,
            first_live_seq: res.next_seq,
            exchanges: 0,
            reported_tokens: 0,
        })
    }

    /// Resume a stored conversation from its last compaction.
    pub async fn load(client: &Client, id: &str, opts: ConversationOptions) -> Result<Self, Error> {
        let mut conv = Self {
            id: id.into(),
            branch: "main".into(),
            title: String::new(),
            client: client.clone(),
            opts,
            history: Vec::new(),
            summary: String::new(),
            next_seq: 0,
            first_live_seq: 0,
            exchanges: 0,
            reported_tokens: 0,
        };
        conv.reload(false, None, None).await?;
        Ok(conv)
    }

    async fn reload(&mut self, full: bool, branch: Option<&str>, at: Option<i64>) -> Result<(), Error> {
        let mut path = format!("/v1/conversations/{}", self.id);
        let mut q = Vec::new();
        if full {
            q.push("full=1".to_string());
        }
        if let Some(b) = branch {
            q.push(format!("branch={}", crate::client::urlencode(b)));
        }
        if let Some(a) = at {
            q.push(format!("at={a}"));
        }
        if !q.is_empty() {
            path = format!("{path}?{}", q.join("&"));
        }
        let res: LoadResponse = self.client.request(reqwest::Method::GET, &path, None).await?;
        self.branch = res.branch;
        self.title = res.title;
        self.next_seq = res.next_seq;
        self.first_live_seq = res.messages.first().map(|m| m.seq).unwrap_or(res.next_seq);
        self.history = res.messages.into_iter().map(|m| m.message).collect();
        self.summary = res.compaction.map(|c| c.summary).unwrap_or_default();
        self.reported_tokens = 0;
        Ok(())
    }

    /// What is held locally: the compaction summary, then live turns.
    pub fn messages(&self) -> Vec<Message> {
        let mut out = Vec::with_capacity(self.history.len() + 1);
        if !self.summary.is_empty() {
            out.push(Message {
                role: "system".into(),
                content: Content::Text(format!("Earlier in this conversation: {}", self.summary)),
                ..Default::default()
            });
        }
        out.extend(self.history.iter().cloned());
        out
    }

    /// The messages to send, guaranteed inside the model's window.
    pub fn for_model(&self) -> Vec<Message> {
        self.fitted()
    }

    /// Memory bearing on the user's message, per the memory mode.
    pub async fn with_context(&self, user_message: &str) -> Result<Option<String>, Error> {
        if self.opts.memory != MemoryMode::Query || user_message.trim().is_empty() {
            return Ok(None);
        }
        self.client.context(user_message, self.opts.namespace.as_deref()).await
    }

    /// Store messages, compacting first when the window or the exchange
    /// cadence demand it. `usage` is the provider response's usage — the real
    /// count, which beats the estimator.
    pub async fn append(&mut self, batch: Vec<Message>, usage: Option<&Usage>) -> Result<(), Error> {
        if batch.is_empty() {
            return Ok(());
        }
        if let Some(u) = usage {
            let t = u.total();
            if t > 0 {
                self.reported_tokens = t;
            }
        }
        self.exchanges += batch.iter().filter(|m| m.role == "assistant").count() as u32;

        if self.opts.summarize.is_some() && (self.would_overflow(&batch) || self.cadence_due()) {
            self.compact().await?;
        }

        let mut wire = Vec::with_capacity(batch.len());
        for m in &batch {
            wire.push(self.to_wire(m).await?);
        }
        #[derive(Deserialize)]
        struct Appended {
            next_seq: i64,
        }
        let res: Appended = self
            .client
            .request(
                reqwest::Method::POST,
                &format!("/v1/conversations/{}/messages", self.id),
                Some(json!({"branch": self.branch, "messages": wire})),
            )
            .await?;
        self.history.extend(batch);
        self.next_seq = res.next_seq;
        Ok(())
    }

    /// Summarize what the window can no longer hold and hand the evicted turns
    /// to memory ingestion.
    pub async fn compact(&mut self) -> Result<Option<String>, Error> {
        let summarize = self.opts.summarize.as_ref().ok_or_else(|| {
            Error::Usage("compaction needs a summarizer; without one the evicted turns would be dropped".into())
        })?;
        let mut evicted_len = self.evictable_len();
        if evicted_len == 0 {
            // The estimator sees room, but the trigger knew better. Evict all
            // but the latest exchange; always at least one when two exist.
            if self.history.len() < 2 {
                return Ok(None);
            }
            let keep = 2.min(self.history.len() - 1);
            evicted_len = self.history.len() - keep;
        }
        let evicted: Vec<Message> = self.history[..evicted_len].to_vec();
        let summary = summarize(&evicted)?;
        let from = self.first_live_seq;
        let to = from + evicted_len as i64 - 1;
        let _: Value = self
            .client
            .request(
                reqwest::Method::POST,
                &format!("/v1/conversations/{}/compact", self.id),
                Some(json!({"branch": self.branch, "summary": summary, "from_seq": from, "to_seq": to})),
            )
            .await?;
        self.summary = if self.summary.is_empty() {
            summary.clone()
        } else {
            format!("{}\n\nThen: {}", self.summary, summary)
        };
        self.history.drain(..evicted_len);
        self.first_live_seq = to + 1;
        self.exchanges = 0;
        self.reported_tokens = 0;
        Ok(Some(summary))
    }

    /// Fork a new branch after `seq` and switch to it. Nothing is deleted.
    pub async fn rewind(&mut self, seq: i64) -> Result<(), Error> {
        #[derive(Deserialize)]
        struct Forked {
            branch: String,
        }
        let res: Forked = self
            .client
            .request(
                reqwest::Method::POST,
                &format!("/v1/conversations/{}/rewind", self.id),
                Some(json!({"to": seq, "branch": self.branch})),
            )
            .await?;
        self.reload(true, Some(&res.branch), Some(seq)).await
    }

    /// Replace the message at `seq` on a NEW branch; the original line is
    /// untouched.
    pub async fn edit(&mut self, seq: i64, replacement: Message) -> Result<(), Error> {
        let wire = self.to_wire(&replacement).await?;
        #[derive(Deserialize)]
        struct Forked {
            branch: String,
        }
        let res: Forked = self
            .client
            .request(
                reqwest::Method::POST,
                &format!("/v1/conversations/{}/edit", self.id),
                Some(json!({"seq": seq, "branch": self.branch, "message": wire})),
            )
            .await?;
        self.reload(true, Some(&res.branch), None).await
    }

    /// Rewrite the message at `seq` in place, destroying the original — the
    /// one edit that does not fork, for content that must stop existing.
    pub async fn edit_in_place(&mut self, seq: i64, content: &str) -> Result<(), Error> {
        let _: Value = self
            .client
            .request(
                reqwest::Method::PATCH,
                &format!("/v1/conversations/{}/messages/{seq}", self.id),
                Some(json!({"branch": self.branch, "content": content})),
            )
            .await?;
        let idx = seq - self.first_live_seq;
        if idx >= 0 && (idx as usize) < self.history.len() {
            self.history[idx as usize].content = Content::Text(content.to_string());
        }
        Ok(())
    }

    /// Name the conversation, overwriting any automatic title.
    pub async fn set_title(&mut self, title: &str) -> Result<(), Error> {
        let _: Value = self
            .client
            .request(
                reqwest::Method::PATCH,
                &format!("/v1/conversations/{}", self.id),
                Some(json!({"title": title})),
            )
            .await?;
        self.title = title.into();
        Ok(())
    }

    /// Every line of this conversation.
    pub async fn branches(&self) -> Result<Vec<BranchInfo>, Error> {
        #[derive(Deserialize)]
        struct Branches {
            #[serde(default)]
            branches: Vec<BranchInfo>,
        }
        let res: Branches = self
            .client
            .request(
                reqwest::Method::GET,
                &format!("/v1/conversations/{}/branches", self.id),
                None,
            )
            .await?;
        Ok(res.branches)
    }

    /// Hand a range of turns to memory without compacting.
    pub async fn ingest(&self, from_seq: i64, to_seq: i64) -> Result<(), Error> {
        let _: Value = self
            .client
            .request(
                reqwest::Method::POST,
                &format!("/v1/conversations/{}/ingest", self.id),
                Some(json!({"branch": self.branch, "from_seq": from_seq, "to_seq": to_seq})),
            )
            .await?;
        Ok(())
    }

    // -- internals --------------------------------------------------------

    /// One message for the wire: parts still carrying bytes are uploaded and
    /// replaced by references; flattened text travels alongside.
    async fn to_wire(&self, m: &Message) -> Result<Value, Error> {
        let mut out = json!({"role": m.role, "content": m.text()});
        if let Some(n) = &m.name {
            out["name"] = json!(n);
        }
        if let Some(tc) = &m.tool_calls {
            out["tool_calls"] = tc.clone();
        }
        if let Some(id) = &m.tool_call_id {
            out["tool_call_id"] = json!(id);
        }
        if let Content::Parts(parts) = &m.content {
            let mut wire_parts = Vec::with_capacity(parts.len());
            for p in parts {
                if let Some(data) = &p.data {
                    let raw = base64::Engine::decode(
                        &base64::engine::general_purpose::STANDARD,
                        &data.base64,
                    )
                    .map_err(|e| Error::Usage(format!("part data is not valid base64: {e}")))?;
                    let info = self.client.upload_media(&data.media_type, &raw).await?;
                    let mut clean = p.clone();
                    clean.data = None;
                    clean.media_id = Some(info.id);
                    wire_parts.push(serde_json::to_value(clean).unwrap());
                } else {
                    wire_parts.push(serde_json::to_value(p).unwrap());
                }
            }
            out["parts"] = Value::Array(wire_parts);
        }
        Ok(out)
    }

    fn budget(&self) -> u32 {
        let ceiling = self
            .opts
            .max_tokens
            .unwrap_or_else(|| (context_limit(&self.opts.model) as f64 * 0.9) as u32);
        ceiling.saturating_sub(self.opts.reserve_for_reply)
    }

    fn fitted(&self) -> Vec<Message> {
        let mut msgs = self.messages();
        let start = usize::from(!self.summary.is_empty());
        while msgs.len() - start > 1 && total_tokens(&msgs) > self.budget() {
            msgs.remove(start);
        }
        msgs
    }

    fn evictable_len(&self) -> usize {
        let fitted_live = self.fitted().len() - usize::from(!self.summary.is_empty());
        self.history.len() - fitted_live
    }

    fn would_overflow(&self, batch: &[Message]) -> bool {
        let threshold = (self.budget() as f64 * self.opts.compact_at.unwrap_or(0.85)) as u32;
        let held = if self.reported_tokens > 0 {
            self.reported_tokens
        } else {
            total_tokens(&self.messages())
        };
        held + total_tokens(batch) > threshold
    }

    fn cadence_due(&self) -> bool {
        let every = self.opts.compact_every.unwrap_or(5);
        every > 0 && self.exchanges >= every
    }
}
