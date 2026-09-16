use base64::Engine as _;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::types::{
    Accepted, MediaInfo, Message, Mode, RecallOptions, RecallResult, Skill, SkillOptions, Term,
};

const DEFAULT_BASE_URL: &str = "https://api.gitloom.cloud";

/// A refusal or failure from the API.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The API refused, with its machine-readable code.
    #[error("gitloom: {message} ({status} {code})")]
    Api {
        status: u16,
        code: String,
        message: String,
    },
    #[error("gitloom: transport: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("gitloom: {0}")]
    Usage(String),
    /// A model-backed mode returned no text. An empty answer must not reach a
    /// caller as an empty string they would show to a user.
    #[error("gitloom: the model did not produce an answer")]
    NoAnswer,
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

    /// Retrieve what is known that bears on the query.
    ///
    /// Every entry is one whole memory. See [`recall_with`] to filter, and
    /// [`answer`] to have a model write the answer instead.
    ///
    /// [`recall_with`]: Client::recall_with
    /// [`answer`]: Client::answer
    pub async fn recall(
        &self,
        query: &str,
        namespace: Option<&str>,
    ) -> Result<RecallResult, Error> {
        let opts = RecallOptions {
            namespace: namespace.map(str::to_string),
            ..Default::default()
        };
        self.recall_with(query, &opts).await
    }

    /// Retrieve with filters. Each one is applied *inside* every retrieval arm
    /// and to graph neighbours server-side, so confining a query to a
    /// directory is a boundary rather than a cut made after the fact.
    pub async fn recall_with(
        &self,
        query: &str,
        opts: &RecallOptions,
    ) -> Result<RecallResult, Error> {
        let ns = opts.namespace.as_deref().unwrap_or(&self.namespace);
        let mut path = format!(
            "/v1/retrieve?q={}&namespace={}",
            urlencode(query),
            urlencode(ns)
        );
        for (key, value) in opts.query_pairs() {
            path.push('&');
            path.push_str(&key);
            path.push('=');
            path.push_str(&urlencode(&value));
        }
        self.request(reqwest::Method::GET, &path, None).await
    }

    /// One text answer drawn from the memory.
    ///
    /// A fast model summarizes one retrieval; with [`Mode::Agentic`] a
    /// stronger model searches the memory itself with tools and returns its
    /// trace. The memories the answer rests on come back on the result. Both
    /// meter as chats rather than reads.
    pub async fn answer(&self, query: &str, opts: &RecallOptions) -> Result<RecallResult, Error> {
        let mut opts = opts.clone();
        if opts.mode != Mode::Agentic {
            opts.mode = Mode::Summary;
        }
        let res = self.recall_with(query, &opts).await?;
        if res.answer.as_deref().unwrap_or("").trim().is_empty() {
            return Err(Error::NoAnswer);
        }
        Ok(res)
    }

    /// Retrieval rendered as one system-message string; None when nothing
    /// relevant is stored.
    pub async fn context(
        &self,
        query: &str,
        namespace: Option<&str>,
    ) -> Result<Option<String>, Error> {
        let res = self.recall(query, namespace).await?;
        if res.memories.is_empty() {
            return Ok(None);
        }
        let mut s = String::from(
            "What you already know about this user, from earlier conversations. \
             Treat it as background, not as something they just said:\n",
        );
        for m in &res.memories {
            let text = if m.content.is_empty() {
                m.snippet.as_deref().unwrap_or("")
            } else {
                &m.content
            };
            s.push_str("- ");
            s.push_str(text);
            s.push('\n');
        }
        Ok(Some(s))
    }

    /// Teach a namespace terms and their aliases. Once learned, a query for
    /// any surface form also finds memories written with another.
    /// Asynchronous, like every write.
    pub async fn learn_terms(
        &self,
        terms: &[Term],
        namespace: Option<&str>,
    ) -> Result<Accepted, Error> {
        let ns = namespace.unwrap_or(&self.namespace);
        let body = serde_json::json!({ "namespace": ns, "terms": terms });
        self.request(reqwest::Method::POST, "/v1/vocab", Some(body))
            .await
    }

    /// The namespace's learned terms, alphabetically.
    pub async fn vocabulary(
        &self,
        like: Option<&str>,
        namespace: Option<&str>,
    ) -> Result<Vec<Term>, Error> {
        #[derive(serde::Deserialize)]
        struct Wrapper {
            #[serde(default)]
            terms: Vec<Term>,
        }
        let ns = namespace.unwrap_or(&self.namespace);
        let mut path = format!("/v1/vocab?namespace={}", urlencode(ns));
        if let Some(like) = like {
            path.push_str(&format!("&like={}", urlencode(like)));
        }
        let w: Wrapper = self.request(reqwest::Method::GET, &path, None).await?;
        Ok(w.terms)
    }

    /// Resolve any surface form to its term. `None` when the word is unknown,
    /// which is not an error.
    pub async fn lookup_term(
        &self,
        word: &str,
        namespace: Option<&str>,
    ) -> Result<Option<Term>, Error> {
        #[derive(serde::Deserialize)]
        struct Wrapper {
            #[serde(default)]
            found: bool,
            term: Option<Term>,
        }
        let ns = namespace.unwrap_or(&self.namespace);
        let path = format!(
            "/v1/vocab?namespace={}&word={}",
            urlencode(ns),
            urlencode(word)
        );
        let w: Wrapper = self.request(reqwest::Method::GET, &path, None).await?;
        Ok(if w.found { w.term } else { None })
    }

    /// Forget terms by canonical form. One never learned is skipped rather
    /// than failing the batch.
    pub async fn forget_terms(
        &self,
        terms: &[&str],
        namespace: Option<&str>,
    ) -> Result<Accepted, Error> {
        let ns = namespace.unwrap_or(&self.namespace);
        let path = format!(
            "/v1/vocab?namespace={}&term={}",
            urlencode(ns),
            urlencode(&terms.join(","))
        );
        self.request(reqwest::Method::DELETE, &path, None).await
    }

    /// Store skills. Each becomes a memory at `skills/<topic>/<slug>.md`; the
    /// returned `paths` say where.
    pub async fn store_skills(
        &self,
        skills: &[Skill],
        namespace: Option<&str>,
    ) -> Result<Accepted, Error> {
        let ns = namespace.unwrap_or(&self.namespace);
        let body = serde_json::json!({ "namespace": ns, "skills": skills });
        self.request(reqwest::Method::POST, "/v1/skills", Some(body))
            .await
    }

    /// The skills that bear on a task, best first. An empty task lists every
    /// skill instead.
    pub async fn find_skills(&self, task: &str, opts: &SkillOptions) -> Result<Vec<Skill>, Error> {
        #[derive(serde::Deserialize)]
        struct Wrapper {
            #[serde(default)]
            skills: Vec<Skill>,
        }
        let ns = opts.namespace.as_deref().unwrap_or(&self.namespace);
        let mut path = format!("/v1/skills?namespace={}", urlencode(ns));
        if !task.is_empty() {
            path.push_str(&format!("&q={}", urlencode(task)));
        }
        if !opts.paths.is_empty() {
            path.push_str(&format!("&paths={}", urlencode(&opts.paths.join(","))));
        }
        if !opts.tags.is_empty() {
            path.push_str(&format!("&tags={}", urlencode(&opts.tags.join(","))));
        }
        if let Some(limit) = opts.limit {
            path.push_str(&format!("&limit={limit}"));
        }
        let w: Wrapper = self.request(reqwest::Method::GET, &path, None).await?;
        Ok(w.skills)
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
        self.request(reqwest::Method::GET, &format!("/v1/media/{id}"), None)
            .await
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
    Error::Api {
        status,
        code,
        message,
    }
}

pub(crate) fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
