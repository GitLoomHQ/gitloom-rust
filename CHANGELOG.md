# Changelog

## 0.2.0 — 2026-08-08

- **`Conversation::exchange`** — the proxy: one call where the SDK does
  everything but the provider request. The closure receives the prepared
  window (memory context and compaction summary folded in); both turns are
  stored afterwards with the provider's usage.
- **Server-side compaction.** `summarize_server: true` hands summarization to
  GitLoom's own model; a local summarizer remains the private-by-default
  choice.

## 0.1.0 — 2026-08-08

- Provider-agnostic conversations with usage-timed compaction, branching,
  edits, redaction, titles, media, and evidenced memory recall; async-openai
  conversions behind the `openai` feature.
