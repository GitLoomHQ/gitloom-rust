# Changelog

## 0.3.0 — 2026-09-16

- **Recall returns memories.** `RecallResult::hits` becomes `memories`, and
  each entry is one whole memory with its `content` rather than a scattering
  of its sections. `matched` names the arms that found it, `sections` the
  headings that matched, `via` what pulled in a neighbour.
- **Scores mean something.** `score` is calibrated in `[0, 1]` and comparable
  across queries, replacing a fused rank that only ordered one response.
  `Scores::coverage` says how much of the query a memory accounted for.
- **`recall_with`** takes filters — `tiers`, `paths`, `tags`, `tags_all`,
  `since`, `until`, `min_score`, `no_context`, `detail`, `include_expired` —
  applied inside every retrieval arm server-side rather than after the fact.
- **`answer`** returns one text answer: a fast model over a retrieval, or with
  `Mode::Agentic` a stronger model that searches the memory itself with tools
  and returns its `trace`. Returns `Error::NoAnswer` rather than an empty
  string. Both meter as chats.
- **Vocabulary** — `learn_terms`, `vocabulary`, `lookup_term`, `forget_terms`.
  A learned alias makes a query for any surface form find memories written
  with another; matched definitions come back as `defined`.
- **Skills** — `store_skills` and `find_skills`, stored as memories under the
  `skills/` tier.
- `candidates`, `filtered_out` and `timings` report what retrieval did.

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
