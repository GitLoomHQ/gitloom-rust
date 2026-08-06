//! Conversions for the `async-openai` crate (feature = "openai").
//!
//! `usage_of` lifts an async-openai response's usage into GitLoom's [`Usage`]
//! so it can time compaction; `messages_of` renders a conversation's fitted
//! window as async-openai request messages.

use async_openai::types::{
    ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
    CreateChatCompletionResponse,
};

use crate::types::{Message, Usage};

/// The response's token usage, in GitLoom's shape.
pub fn usage_of(res: &CreateChatCompletionResponse) -> Usage {
    let u = res.usage.as_ref();
    Usage {
        prompt_tokens: u.map(|u| u.prompt_tokens),
        completion_tokens: u.map(|u| u.completion_tokens),
        total_tokens: u.map(|u| u.total_tokens),
        ..Default::default()
    }
}

/// A conversation window as async-openai request messages. Text only — parts
/// with media are flattened to their text; replaying stored attachments into
/// a new completion is the caller's decision, made with `Client::get_media`.
pub fn messages_of(window: &[Message]) -> Vec<ChatCompletionRequestMessage> {
    window
        .iter()
        .filter_map(|m| {
            let text = m.text();
            match m.role.as_str() {
                "system" => ChatCompletionRequestSystemMessageArgs::default()
                    .content(text)
                    .build()
                    .ok()
                    .map(Into::into),
                "assistant" => ChatCompletionRequestAssistantMessageArgs::default()
                    .content(text)
                    .build()
                    .ok()
                    .map(Into::into),
                _ => ChatCompletionRequestUserMessageArgs::default()
                    .content(text)
                    .build()
                    .ok()
                    .map(Into::into),
            }
        })
        .collect()
}
