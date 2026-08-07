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

/// One retrieved memory with its evidence — the shape every GitLoom surface
/// returns.
#[derive(Debug, Clone, Deserialize)]
pub struct Hit {
    pub path: String,
    pub score: f64,
    pub snippet: String,
    pub scores: Option<Scores>,
    pub provenance: Option<Provenance>,
    #[serde(default)]
    pub relations: Vec<Relation>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Scores {
    pub bm25: Option<f64>,
    pub cue: Option<f64>,
    pub body: Option<f64>,
    pub graph_hops: Option<i64>,
    #[serde(default)]
    pub arms: Vec<String>,
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

#[derive(Debug, Clone, Deserialize)]
pub struct RecallResult {
    pub namespace: String,
    #[serde(default)]
    pub hits: Vec<Hit>,
    pub millis: i64,
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
