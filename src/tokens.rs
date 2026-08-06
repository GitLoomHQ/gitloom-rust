//! Estimation, biased to overcount; provider [`crate::Usage`] overrides it.

use crate::types::{Content, Message};

const CONTEXT_LIMITS: &[(&str, u32)] = &[
    ("claude-opus-5", 1_000_000),
    ("claude-sonnet-5", 1_000_000),
    ("claude-fable-5", 1_000_000),
    ("claude-haiku-4-5", 200_000),
    ("claude-", 200_000),
    ("gpt-5", 400_000),
    ("gpt-4.1", 1_047_576),
    ("gpt-4o", 128_000),
    ("gpt-4", 8_192),
    ("o1", 200_000),
    ("o3", 200_000),
    ("gemini-1.5-pro", 2_000_000),
    ("gemini-", 1_000_000),
    ("llama-3", 128_000),
    ("mistral-", 32_000),
];

const CHARS_PER_TOKEN: f64 = 3.5;
const PER_MESSAGE_OVERHEAD: u32 = 4;
const PER_REQUEST_OVERHEAD: u32 = 3;
const PER_IMAGE_TOKENS: u32 = 1_100;

/// The model's input window, by longest matching prefix.
pub fn context_limit(model: &str) -> u32 {
    let m = model.to_lowercase();
    let mut best = 0usize;
    let mut limit = 128_000;
    for (prefix, value) in CONTEXT_LIMITS {
        if m.starts_with(prefix) && prefix.len() >= best {
            best = prefix.len();
            limit = *value;
        }
    }
    limit
}

/// Estimate tokens in a string — a ratio, not a tokenizer, absorbed by the
/// safety margin.
pub fn estimate_tokens(text: &str) -> u32 {
    if text.is_empty() {
        return 0;
    }
    (text.len() as f64 / CHARS_PER_TOKEN) as u32 + 1
}

/// Tokens in one message, including structural overhead. Non-text parts are
/// billed as images — audio and documents cost at least as much.
pub fn message_tokens(m: &Message) -> u32 {
    let mut n = PER_MESSAGE_OVERHEAD;
    match &m.content {
        Content::Text(s) => n += estimate_tokens(s),
        Content::Parts(parts) => {
            for p in parts {
                match &p.text {
                    Some(t) => n += estimate_tokens(t),
                    None => n += PER_IMAGE_TOKENS,
                }
            }
        }
    }
    if let Some(tc) = &m.tool_calls {
        n += estimate_tokens(&tc.to_string());
    }
    n
}

/// Tokens in a whole conversation.
pub fn total_tokens(messages: &[Message]) -> u32 {
    PER_REQUEST_OVERHEAD + messages.iter().map(message_tokens).sum::<u32>()
}
