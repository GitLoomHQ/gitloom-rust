use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One chat message, in the provider's shape. `content` is either text or an
/// array of content parts (text, image, audio blocks).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Message {
    pub role: String,
    #[serde(default)]
    pub content: Content,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: Content::Text(text.into()),
            ..Default::default()
        }
    }
    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: Content::Text(text.into()),
            ..Default::default()
        }
    }
    /// The flattened text, whatever shape the content takes.
    pub fn text(&self) -> String {
        match &self.content {
            Content::Text(s) => s.clone(),
            Content::Parts(parts) => parts
                .iter()
                .filter_map(|p| p.text.as_deref())
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

/// Message content: plain text or multimodal parts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Parts(Vec<Part>),
}

impl Default for Content {
    fn default() -> Self {
        Content::Text(String::new())
    }
}

/// One block of a multimodal message. Deliberately loose: whatever the
/// provider emitted is stored and replayed verbatim. `media_id` and `data` are
/// GitLoom's additions — `data` is uploaded transparently on append and
/// replaced by a `media_id` reference.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Part {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<PartData>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

/// Bytes to upload on append.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartData {
    pub base64: String,
    pub media_type: String,
}

impl Part {
    pub fn text_part(text: impl Into<String>) -> Self {
        Self {
            kind: "text".into(),
            text: Some(text.into()),
            ..Default::default()
        }
    }
    pub fn image(media_id: impl Into<String>) -> Self {
        Self {
            kind: "image".into(),
            media_id: Some(media_id.into()),
            ..Default::default()
        }
    }
    pub fn image_data(base64: impl Into<String>, media_type: impl Into<String>) -> Self {
        Self {
            kind: "image".into(),
            data: Some(PartData {
                base64: base64.into(),
                media_type: media_type.into(),
            }),
            ..Default::default()
        }
    }
}

/// Token usage as providers report it — either spelling.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Usage {
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
}

impl Usage {
    pub fn total(&self) -> u32 {
        if let Some(t) = self.total_tokens {
            return t;
        }
        self.prompt_tokens.or(self.input_tokens).unwrap_or(0)
            + self.completion_tokens.or(self.output_tokens).unwrap_or(0)
    }
}

/// One retrieved memory — the whole memory, not a fragment of one. A memory
/// whose sections matched separately is still one entry, with the matching
/// sections named.
#[derive(Debug, Clone, Deserialize)]
pub struct Memory {
    pub path: String,
    #[serde(default)]
    pub tier: String,
    pub topic: Option<String>,
    pub title: Option<String>,
    #[serde(default)]
    pub content: String,
    /// The query-focused excerpt, when the lexical arm matched.
    pub snippet: Option<String>,

    /// A calibrated relevance in `[0, 1]`, comparable *across* queries: a
    /// memory that answers the question outright scores near 1 whatever else
    /// the namespace holds. It replaced a fused rank that only meant
    /// something within one response.
    pub score: f64,
    /// Which arms produced this memory: `lexical`, `cue`, `body`, `graph`.
    /// Matched only by `graph` means context that rode in beside a real
    /// match rather than evidence, and `via` names what pulled it in.
    #[serde(default)]
    pub matched: Vec<String>,
    #[serde(default)]
    pub sections: Vec<String>,
    #[serde(default)]
    pub via: Vec<String>,

    #[serde(default)]
    pub tags: Vec<String>,
    pub created: Option<String>,
    pub updated: Option<String>,
    pub confidence: Option<f64>,
    #[serde(default)]
    pub cues: Vec<String>,

    pub scores: Option<Scores>,
    #[serde(default)]
    pub related: Vec<Relation>,
    pub provenance: Option<Provenance>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Scores {
    pub bm25: Option<f64>,
    pub cue: Option<f64>,
    pub body: Option<f64>,
    pub graph_hops: Option<i64>,
    /// The fraction of the query's content terms the memory holds.
    pub coverage: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Provenance {
    pub commit: String,
    pub author: Option<String>,
    pub when: String,
    pub message: Option<String>,
    pub revisions: Option<i64>,
    #[serde(default)]
    pub history: Vec<Revision>,
    pub diff: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Revision {
    pub commit: String,
    pub when: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Relation {
    pub label: Option<String>,
    pub path: String,
    pub snippet: Option<String>,
    pub valid_from: Option<i64>,
    pub valid_to: Option<i64>,
}

/// A custom-vocabulary term the query matched.
#[derive(Debug, Clone, Deserialize)]
pub struct VocabHit {
    pub path: String,
    pub term: String,
    pub definition: Option<String>,
    #[serde(default)]
    pub matched: Vec<String>,
}

/// One step of an agentic retrieval's trace.
#[derive(Debug, Clone, Deserialize)]
pub struct TraceEvent {
    #[serde(rename = "type")]
    pub kind: String,
    pub tool: Option<String>,
    pub id: Option<String>,
    pub input: Option<serde_json::Value>,
    pub result: Option<serde_json::Value>,
    pub text: Option<String>,
    pub millis: Option<i64>,
}

/// Where a retrieval spent its time. `model_ms` is set only when a mode ran
/// one.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Timings {
    #[serde(default)]
    pub lexical_ms: i64,
    #[serde(default)]
    pub vector_ms: i64,
    #[serde(default)]
    pub graph_ms: i64,
    pub model_ms: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RecallResult {
    pub namespace: String,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub memories: Vec<Memory>,
    #[serde(default)]
    pub defined: Vec<VocabHit>,

    /// Set by [`Mode::Summary`] and [`Mode::Agentic`]. `truncated` means the
    /// agent hit its budget before choosing to stop.
    pub answer: Option<String>,
    pub model: Option<String>,
    #[serde(default)]
    pub trace: Vec<TraceEvent>,
    #[serde(default)]
    pub truncated: bool,

    /// How many distinct memories any arm produced before the relevance
    /// floor, and how many that floor dropped. Many filtered out with no
    /// memories is an unanswerable question rather than a miss.
    #[serde(default)]
    pub candidates: i64,
    #[serde(default)]
    pub filtered_out: i64,
    #[serde(default)]
    pub millis: i64,
    #[serde(default)]
    pub timings: Timings,
}

/// A vocabulary entry: a canonical form, the surface forms that mean the same
/// thing, and what it means.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Term {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub term: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
}

/// Procedural know-how, stored as an ordinary memory under the skills tier.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Skill {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The procedure, as markdown; `##` headings become sections.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// How someone would ask for this skill. These become its retrieval cues,
    /// so write them as the question rather than the topic. Empty falls back
    /// to the name and description.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,

    /// Set on results, not on input.
    #[serde(default, skip_serializing)]
    pub score: f64,
    #[serde(default, skip_serializing)]
    pub matched: Vec<String>,
}

/// The acknowledgement every asynchronous write returns.
#[derive(Debug, Clone, Deserialize)]
pub struct Accepted {
    pub id: String,
    pub namespace: String,
    pub status: String,
    #[serde(default)]
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MediaInfo {
    pub id: String,
    pub content_type: Option<String>,
    pub bytes: Option<i64>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BranchInfo {
    pub name: String,
    pub forked_from: Option<String>,
    pub forked_at: Option<i64>,
}

/// How a retrieval answers. `Raw` makes no model call beyond the query
/// embedding and meters as a read; the other two run a model and meter as a
/// chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Raw,
    Summary,
    Agentic,
}

impl Mode {
    fn as_param(self) -> Option<&'static str> {
        match self {
            Mode::Raw => None,
            Mode::Summary => Some("summary"),
            Mode::Agentic => Some("agentic"),
        }
    }
}

/// Filters for a retrieval. Every one is applied *inside* each retrieval arm
/// and to graph neighbours server-side.
#[derive(Debug, Clone, Default)]
pub struct RecallOptions {
    pub namespace: Option<String>,
    pub limit: Option<u32>,
    pub mode: Mode,
    /// `facts`, `incidents`, `rules`, `skills`.
    pub tiers: Vec<String>,
    /// Directories, e.g. `facts/events`.
    pub paths: Vec<String>,
    /// Any of these tags.
    pub tags: Vec<String>,
    /// Every one of these tags.
    pub tags_all: Vec<String>,
    /// `YYYY-MM-DD` or RFC 3339, over the memory's `updated`.
    pub since: Option<String>,
    pub until: Option<String>,
    /// Drop memories scoring below this.
    pub min_score: Option<f64>,
    /// Drop graph neighbours, leaving only what matched the query directly.
    pub no_context: bool,
    /// `full` adds revision history, the last diff, relation snippets and cues.
    pub detail: Option<String>,
    pub include_expired: bool,
}

impl RecallOptions {
    /// The parameters that differ from the defaults. Nothing is sent for a
    /// field left unset, so a default request carries only `q` and
    /// `namespace`.
    pub(crate) fn query_pairs(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut push = |k: &str, v: String| out.push((k.to_string(), v));
        if let Some(limit) = self.limit {
            push("limit", limit.to_string());
        }
        if let Some(mode) = self.mode.as_param() {
            push("mode", mode.to_string());
        }
        for (key, values) in [
            ("tiers", &self.tiers),
            ("paths", &self.paths),
            ("tags", &self.tags),
            ("tags_all", &self.tags_all),
        ] {
            if !values.is_empty() {
                push(key, values.join(","));
            }
        }
        if let Some(since) = &self.since {
            push("since", since.clone());
        }
        if let Some(until) = &self.until {
            push("until", until.clone());
        }
        if let Some(min) = self.min_score {
            push("min_score", min.to_string());
        }
        if self.no_context {
            push("context", "0".to_string());
        }
        if let Some(detail) = &self.detail {
            push("detail", detail.clone());
        }
        if self.include_expired {
            push("include_expired", "1".to_string());
        }
        out
    }
}

/// Narrows a skill search.
#[derive(Debug, Clone, Default)]
pub struct SkillOptions {
    pub namespace: Option<String>,
    /// Topics under `skills/`, e.g. `ops`.
    pub paths: Vec<String>,
    pub tags: Vec<String>,
    pub limit: Option<u32>,
}
