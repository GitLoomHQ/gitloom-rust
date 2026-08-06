//! GitLoom: conversations that cannot outgrow their context window, backed by
//! a memory the model can consult.
//!
//! The core is provider-agnostic — messages are [`Message`] values that mirror
//! the OpenAI/Anthropic chat shape, and compaction is timed by the [`Usage`]
//! you pass from your provider's response. The `openai` feature adds
//! conversions for the `async-openai` crate.

mod client;
mod conversation;
mod tokens;
mod types;

#[cfg(feature = "openai")]
pub mod openai;

pub use client::{Client, Error};
pub use conversation::{Conversation, ConversationOptions, MemoryMode, Summarizer};
pub use tokens::{context_limit, estimate_tokens, message_tokens, total_tokens};
pub use types::*;
