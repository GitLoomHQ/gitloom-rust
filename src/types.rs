use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize};
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
    /// Which arms produced this memory: `lexical`, `cue`, `body`, `graph`;
    /// on the lane path, also `time`. Matched only by `graph` means context
    /// that rode in beside a real match rather than evidence, and `via` names
    /// what pulled it in.
    #[serde(default)]
    pub matched: Vec<String>,
    #[serde(default)]
    pub sections: Vec<String>,
    #[serde(default)]
    pub via: Vec<String>,

    /// Lane path: a curated `memory`, or a conversation `turn` kept word for
    /// word.
    pub store: Option<String>,
    /// Lane path: the days it was said, `YYYY-MM-DD`, oldest first.
    #[serde(default)]
    pub said: Vec<String>,
    /// `content` was cut to fit `max_chars`.
    #[serde(default)]
    pub excerpted: bool,

    /// The caller's tags first, then the inferred ones.
    #[serde(default)]
    pub tags: Vec<String>,
    /// The caller's tags alone.
    #[serde(default)]
    pub user_tags: Vec<String>,
    #[deprecated(note = "use created_at")]
    pub created: Option<String>,
    #[deprecated(note = "use updated_at")]
    pub updated: Option<String>,
    #[serde(default, deserialize_with = "unix_seconds")]
    pub created_at: Option<SystemTime>,
    #[serde(default, deserialize_with = "unix_seconds")]
    pub updated_at: Option<SystemTime>,
    /// When the memory's subject happened.
    #[serde(default, deserialize_with = "unix_seconds")]
    pub occurred_at: Option<SystemTime>,
    #[serde(default, deserialize_with = "unix_seconds")]
    pub expires_at: Option<SystemTime>,
    /// How `occurred_at` is known: `user`, `extracted`, `said` or `written`.
    pub occurred_source: Option<String>,
    /// `instant`, or `day` when only the date is known, held as noon UTC on
    /// that date.
    pub occurred_precision: Option<String>,
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
    /// Lane path: the query embedding, every lane, the ranking call, and each
    /// lane on each store.
    pub embed_ms: Option<i64>,
    pub lanes_ms: Option<i64>,
    pub rank_ms: Option<i64>,
    #[serde(default)]
    pub lane: Vec<LaneTiming>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LaneTiming {
    #[serde(default)]
    pub lane: String,
    #[serde(default)]
    pub store: String,
    #[serde(default)]
    pub ms: i64,
    #[serde(default)]
    pub n: i64,
    pub err: Option<String>,
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

    /// The lane ranking asked for. `rank_fallback` means `Rank::Jev` could
    /// not rank, so the memories are in lane order.
    pub rank: Option<String>,
    #[serde(default)]
    pub rank_fallback: bool,

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

/// Shapes one conversation write.
#[derive(Debug, Clone, Default)]
pub struct RememberOptions {
    pub namespace: Option<String>,
    pub session_id: Option<String>,
    /// Applied to every memory drawn from the conversation.
    pub tags: Vec<String>,
    /// When the conversation happened.
    pub occurred_at: Option<Timestamp>,
    /// The IANA zone, e.g. `Asia/Kolkata`, that a time without an offset is
    /// read in.
    pub timezone: Option<String>,
    #[deprecated(note = "use occurred_at")]
    pub date: Option<String>,
}

/// One already-formed memory to store — the input to
/// [`Client::write`](crate::Client::write), as [`Memory`] is the output of a
/// recall.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewMemory {
    /// Repo-relative and ending in `.md`, under `facts/`, `incidents/`,
    /// `rules/` or `skills/`. The directory is the topic.
    pub path: String,
    /// Markdown; `##` headings become separately retrievable sections.
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// When what the memory is about happened, not when it was written.
    /// Backfilled memories without it are all stamped with today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<Timestamp>,
    #[deprecated(note = "use occurred_at")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// In `[0, 1]`; breaks ties between memories that contradict each other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// Expires an incident, e.g. `30d`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl: Option<String>,
    /// A memory this one replaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    /// Two to five ways someone would later ask for this.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cues: Vec<String>,
    /// Paths of related memories, optionally labelled
    /// (`spouse: facts/people/maya.md`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
}

/// One memory read back by path.
#[derive(Debug, Clone, Deserialize)]
pub struct StoredMemory {
    #[serde(default)]
    pub namespace: String,
    pub path: String,
    pub title: Option<String>,
    pub tier: Option<String>,
    pub kind: Option<String>,
    #[serde(default)]
    pub content: String,
    /// The caller's tags first, then the inferred ones.
    #[serde(default, deserialize_with = "nullable")]
    pub tags: Vec<String>,
    /// The caller's tags alone.
    #[serde(default, deserialize_with = "nullable")]
    pub user_tags: Vec<String>,
    pub confidence: Option<f64>,
    #[serde(default, deserialize_with = "nullable")]
    pub cues: Vec<String>,
    #[serde(default, deserialize_with = "nullable")]
    pub related: Vec<String>,
    #[deprecated(note = "use created_at")]
    pub created: Option<String>,
    #[deprecated(note = "use updated_at")]
    pub updated: Option<String>,
    #[serde(default, deserialize_with = "unix_seconds")]
    pub created_at: Option<SystemTime>,
    #[serde(default, deserialize_with = "unix_seconds")]
    pub updated_at: Option<SystemTime>,
    #[serde(default, deserialize_with = "unix_seconds")]
    pub occurred_at: Option<SystemTime>,
    #[serde(default, deserialize_with = "unix_seconds")]
    pub expires_at: Option<SystemTime>,
    /// How `occurred_at` is known: `user`, `extracted`, `said` or `written`.
    pub occurred_source: Option<String>,
    /// `instant`, or `day` when only the date is known.
    pub occurred_precision: Option<String>,
}

/// Roots and bounds a table of contents.
#[derive(Debug, Clone, Default)]
pub struct TreeOptions {
    pub namespace: Option<String>,
    /// Unset is the whole memory.
    pub path: Option<String>,
    /// Default 2, at most 8.
    pub depth: Option<u32>,
}

/// One level of the table of contents: tier, topic, file, section.
#[derive(Debug, Clone, Deserialize)]
pub struct TreeNode {
    #[serde(default)]
    pub path: String,
    pub title: Option<String>,
    pub kind: Option<String>,
    pub tier: Option<String>,
    pub summary: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    pub children: Vec<TreeNode>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TreeResult {
    #[serde(default)]
    pub namespace: String,
    #[serde(default)]
    pub depth: i64,
    pub tree: TreeNode,
    #[serde(default)]
    pub millis: i64,
}

/// Narrows a topic listing.
#[derive(Debug, Clone, Default)]
pub struct TopicsOptions {
    pub namespace: Option<String>,
    /// `facts`, `incidents`, `rules` or `skills`.
    pub tier: Option<String>,
    /// Only topics under this path.
    pub prefix: Option<String>,
    /// A case-insensitive substring of the topic's name.
    pub like: Option<String>,
    pub max_depth: Option<u32>,
    pub min_files: Option<u32>,
    pub limit: Option<u32>,
}

/// One directory in the memory, with how many memories it holds.
#[derive(Debug, Clone, Deserialize)]
pub struct Topic {
    pub path: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub tier: String,
    #[serde(default)]
    pub parent: String,
    #[serde(default)]
    pub depth: i64,
    #[serde(default)]
    pub memories: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TopicsResult {
    #[serde(default)]
    pub namespace: String,
    #[serde(default, deserialize_with = "nullable")]
    pub topics: Vec<Topic>,
    #[serde(default)]
    pub millis: i64,
}

#[derive(Debug, Clone, Default)]
pub struct GraphOptions {
    pub namespace: Option<String>,
    pub limit: Option<u32>,
}

/// One memory in the relationship graph.
#[derive(Debug, Clone, Deserialize)]
pub struct GraphNode {
    pub path: String,
    #[serde(default)]
    pub tier: String,
    #[serde(default)]
    pub kind: String,
    pub title: Option<String>,
}

/// One declared relationship between two memories.
#[derive(Debug, Clone, Deserialize)]
pub struct GraphEdge {
    pub src: String,
    pub dst: String,
    pub label: Option<String>,
    pub origin: Option<String>,
}

/// `truncated` means the graph was larger than one response.
#[derive(Debug, Clone, Deserialize)]
pub struct GraphResult {
    #[serde(default)]
    pub namespace: String,
    #[serde(default, deserialize_with = "nullable")]
    pub nodes: Vec<GraphNode>,
    #[serde(default, deserialize_with = "nullable")]
    pub edges: Vec<GraphEdge>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub millis: i64,
}

fn nullable<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
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

/// How the lane path orders what it finds: by lane score, or with a ranking
/// model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rank {
    Fused,
    Jev,
}

impl Rank {
    fn as_param(self) -> &'static str {
        match self {
            Rank::Fused => "fused",
            Rank::Jev => "jev",
        }
    }
}

/// A model that can read the memories in [`Mode::Summary`] or
/// [`Mode::Agentic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReaderModel {
    Haiku,
    Sonnet,
}

impl ReaderModel {
    fn as_param(self) -> &'static str {
        match self {
            ReaderModel::Haiku => "haiku",
            ReaderModel::Sonnet => "sonnet",
        }
    }
}

/// A time as the API reads one: epoch seconds, or a string sent as-is — RFC
/// 3339 with an offset, a date `YYYY-MM-DD` meaning that calendar day, or a
/// datetime without an offset, read in the request's zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Timestamp {
    Epoch(i64),
    Text(String),
}

impl Timestamp {
    fn as_param(&self) -> String {
        match self {
            Timestamp::Epoch(s) => s.to_string(),
            Timestamp::Text(s) => s.clone(),
        }
    }
}

/// Floored to whole seconds. The server reads a number as epoch seconds only
/// from 9 to 11 digits, so a time before 1973-03-03 or after the year 5138
/// goes as an RFC 3339 UTC string instead.
impl From<SystemTime> for Timestamp {
    fn from(t: SystemTime) -> Self {
        let secs = match t.duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_secs() as i64,
            Err(e) => {
                let d = e.duration();
                0i64.saturating_sub_unsigned(d.as_secs())
                    .saturating_sub(i64::from(d.subsec_nanos() > 0))
            }
        };
        if (100_000_000..100_000_000_000).contains(&secs) {
            Timestamp::Epoch(secs)
        } else {
            Timestamp::Text(rfc3339_utc(secs))
        }
    }
}

fn rfc3339_utc(secs: i64) -> String {
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        s / 3600,
        s / 60 % 60,
        s % 60
    )
}

/// Howard Hinnant's civil_from_days: days since 1970-01-01 to (year, month,
/// day) in the proleptic Gregorian calendar.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

impl From<i64> for Timestamp {
    fn from(secs: i64) -> Self {
        Timestamp::Epoch(secs)
    }
}

impl From<&str> for Timestamp {
    fn from(s: &str) -> Self {
        Timestamp::Text(s.to_string())
    }
}

impl From<String> for Timestamp {
    fn from(s: String) -> Self {
        Timestamp::Text(s)
    }
}

/// Which of a memory's times a range bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeField {
    /// When the memory's subject happened.
    Occurred,
    Created,
    Updated,
}

impl TimeField {
    fn as_param(self) -> &'static str {
        match self {
            TimeField::Occurred => "occurred",
            TimeField::Created => "created",
            TimeField::Updated => "updated",
        }
    }
}

fn unix_seconds<'de, D: Deserializer<'de>>(d: D) -> Result<Option<SystemTime>, D::Error> {
    Ok(Option::<i64>::deserialize(d)?.and_then(|s| {
        let d = Duration::from_secs(s.unsigned_abs());
        if s >= 0 {
            UNIX_EPOCH.checked_add(d)
        } else {
            UNIX_EPOCH.checked_sub(d)
        }
    }))
}

/// Filters for a retrieval. Every one is applied *inside* each retrieval arm
/// and to graph neighbours server-side. With no query, the filters list every
/// memory they match instead, newest first by `time_field`.
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
    /// Bounds on `time_field`. A date-only `until` includes that whole day in
    /// `tz`.
    pub since: Option<Timestamp>,
    pub until: Option<Timestamp>,
    /// Which time `since` and `until` bound, and which a query-less listing
    /// orders by. The server's default is [`TimeField::Updated`].
    pub time_field: Option<TimeField>,
    /// The IANA zone, e.g. `Asia/Kolkata`, that dates and times without an
    /// offset are read in.
    pub tz: Option<String>,
    /// Drop memories scoring below this.
    pub min_score: Option<f64>,
    /// Drop graph neighbours, leaving only what matched the query directly.
    pub no_context: bool,
    /// `full` adds revision history, the last diff, relation snippets and cues.
    pub detail: Option<String>,
    pub include_expired: bool,
    /// Retrieve on the lane path, which also reaches conversation turns and
    /// the dates in a question. Not with [`Mode::Agentic`].
    pub rank: Option<Rank>,
    /// The most characters of memory content to return; memories that do not
    /// fit come back `excerpted`.
    pub max_chars: Option<u32>,
    pub model: Option<ReaderModel>,
}

impl RecallOptions {
    /// Whether a filter says what to list when there is no query.
    pub(crate) fn has_filter(&self) -> bool {
        !self.tags.is_empty()
            || !self.tags_all.is_empty()
            || self.since.is_some()
            || self.until.is_some()
            || !self.tiers.is_empty()
            || !self.paths.is_empty()
    }

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
            push("since", since.as_param());
        }
        if let Some(until) = &self.until {
            push("until", until.as_param());
        }
        if let Some(field) = self.time_field {
            push("time_field", field.as_param().to_string());
        }
        if let Some(tz) = &self.tz {
            push("tz", tz.clone());
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
        if let Some(rank) = self.rank {
            push("rank", rank.as_param().to_string());
        }
        if let Some(max) = self.max_chars.filter(|&n| n > 0) {
            push("max_chars", max.to_string());
        }
        if let Some(model) = self.model {
            push("model", model.as_param().to_string());
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

#[cfg(test)]
mod tests {
    use super::{civil_from_days, rfc3339_utc};

    #[test]
    fn civil_dates_round_the_leap_day() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(11_017), (2000, 3, 1));
        assert_eq!(rfc3339_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339_utc(951_868_799), "2000-02-29T23:59:59Z");
        assert_eq!(rfc3339_utc(-1), "1969-12-31T23:59:59Z");
    }
}
