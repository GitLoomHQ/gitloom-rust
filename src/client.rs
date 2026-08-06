use base64::Engine as _;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::types::{MediaInfo, Message, RecallResult};

const DEFAULT_BASE_URL: &str = "https://api.gitloom.cloud";

/// A refusal or failure from the API.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The API refused, with its machine-readable code.
    #[error("gitloom: {message} ({status} {code})")]
    Api { status: u16, code: String, message: String },
    #[error("gitloom: transport: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("gitloom: {0}")]
    Usage(String),
}

/// The GitLoom client. Writes are never retried: a retried write that
/// half-succeeded double-charges the meter and double-stores the message.
#[derive(Clone)]
pub struct Client {
    pub(crate) http: reqwest::Client,
    pub(crate) base_url: String,
    pub(crate) api_key: String,
    pub(crate) namespace: String,
}

impl Client {
    /// An empty key falls back to `GITLOOM_API_KEY`.
    pub fn new(api_key: impl Into<String>) -> Self {
        let mut key = api_key.into();
        if key.is_empty() {
            key = std::env::var("GITLOOM_API_KEY").unwrap_or_default();
        }
        Self {
            http: reqwest::Client::new(),
            base_url: DEFAULT_BASE_URL.into(),
            api_key: key,
            namespace: "default".into(),
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into().trim_end_matches('/').to_string();
        self
    }

    pub fn with_namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace = ns.into();
        self
    }

    pub(crate) async fn request<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<T, Error> {
        let mut req = self
            .http
            .request(method, format!("{}{}", self.base_url, path))
            .bearer_auth(&self.api_key);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req.send().await?;
        let status = res.status().as_u16();
        let raw = res.bytes().await?;
        if status >= 400 {
            return Err(error_from(status, &raw));
        }
        serde_json::from_slice(&raw).map_err(|e| Error::Api {
            status,
            code: "bad_response".into(),
            message: format!("undecodable JSON: {e}"),
        })
    }

    /// Submit a conversation for ingestion. Asynchronous by design.
    pub async fn remember(&self, turns: &[Message], namespace: Option<&str>) -> Result<(), Error> {
        let turns: Vec<Value> = turns
            .iter()
            .map(|m| json!({"role": m.role, "content": m.text()}))
            .collect();
        let _: Value = self
            .request(
                reqwest::Method::POST,
                "/v1/memories",
                Some(json!({
                    "namespace": namespace.unwrap_or(&self.namespace),
                    "messages": turns,
                })),
            )
            .await?;
        Ok(())
    }

    /// Retrieve what is known that bears on the query. Every hit carries its
    /// evidence: per-arm scores, git history with the last diff, relations.
    pub async fn recall(&self, query: &str, namespace: Option<&str>) -> Result<RecallResult, Error> {
        let ns = namespace.unwrap_or(&self.namespace);
        let path = format!(
            "/v1/retrieve?q={}&namespace={}",
            urlencode(query),
            urlencode(ns)
        );
        self.request(reqwest::Method::GET, &path, None).await
    }

    /// Retrieval rendered as one system-message string; None when nothing
    /// relevant is stored.
    pub async fn context(&self, query: &str, namespace: Option<&str>) -> Result<Option<String>, Error> {
        let res = self.recall(query, namespace).await?;
        if res.hits.is_empty() {
            return Ok(None);
        }
        let mut s = String::from(
            "What you already know about this user, from earlier conversations. \
             Treat it as background, not as something they just said:\n",
        );
        for h in &res.hits {
            s.push_str("- ");
            s.push_str(&h.snippet);
            s.push('\n');
        }
        Ok(Some(s))
    }

    /// Store one attachment (10MB cap; images, audio, PDF, text).
    pub async fn upload_media(&self, content_type: &str, data: &[u8]) -> Result<MediaInfo, Error> {
        self.request(
            reqwest::Method::POST,
            "/v1/media",
            Some(json!({
                "content_type": content_type,
                "data": base64::engine::general_purpose::STANDARD.encode(data),
            })),
        )
        .await
    }

    /// The attachment's description plus a short-lived URL for its bytes.
    pub async fn get_media(&self, id: &str) -> Result<MediaInfo, Error> {
        self.request(reqwest::Method::GET, &format!("/v1/media/{id}"), None).await
    }

    /// Make a namespace exist. Idempotent.
    pub async fn create_namespace(&self, name: &str) -> Result<(), Error> {
        let _: Value = self
            .request(
                reqwest::Method::POST,
                "/v1/namespaces",
                Some(json!({"namespace": name})),
            )
            .await?;
        Ok(())
    }
}

/// Decodes both error shapes the API uses: the {code, message} envelope and
/// the flat {"error": "..."} of the retrieval routes.
fn error_from(status: u16, raw: &[u8]) -> Error {
    let mut code = "http_error".to_string();
    let mut message = String::from_utf8_lossy(raw).trim().to_string();
    if let Ok(v) = serde_json::from_slice::<Value>(raw) {
        match v.get("error") {
            Some(Value::String(s)) => message = s.clone(),
            Some(Value::Object(o)) => {
                if let Some(c) = o.get("code").and_then(Value::as_str) {
                    code = c.into();
                }
                if let Some(m) = o.get("message").and_then(Value::as_str) {
                    message = m.into();
                }
            }
            _ => {}
        }
    }
    Error::Api { status, code, message }
}

pub(crate) fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
