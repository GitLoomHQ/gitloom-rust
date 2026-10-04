# Changelog

## 0.4.0 — unreleased

- **Direct memory primitives**, at parity with the Go SDK: `write` stores
  already-formed `NewMemory` values (refusing a path that does not end in `.md`
  before sending), `get` reads one by path, `forget` deletes by path, and
  `tree`, `topics` and `graph` navigate the table of contents, the topic
  directories and the relationship graph.
- **Tags and times on writes.** `remember_with` takes `RememberOptions`:
  `tags` for every memory drawn from the conversation, `occurred_at`,
  `timezone` and `session_id`. `NewMemory` takes `tags` and `occurred_at`.
  `occurred_at` is a `Timestamp`, built from a `SystemTime` (epoch seconds,
  or RFC 3339 UTC outside the 9-to-11-digit range the server reads as epoch
  seconds), an integer (sent as given) or a string (sent as-is). `date` still
  works and is deprecated.
- **Recall by filter alone.** An empty query lists every memory the filters
  match, newest first; with neither a query nor a filter, `recall_with`
  returns `Error::Usage` without calling the server. `RecallOptions` gains
  `time_field` (`TimeField::Occurred`, `Created` or `Updated`) and `tz`, and
  `since` and `until` become a `Timestamp` — a `&str` still converts with
  `.into()`.
- **Memory times.** `Memory` carries `user_tags`, `created_at`, `updated_at`,
  `occurred_at` and `expires_at` as `Option<SystemTime>`, `occurred_source` and
  `occurred_precision`. `created` and `updated` are deprecated.
- **`recall_with` and `answer` take `rank`, `max_chars` and `model`.** `rank`
  (`Rank::Fused` or `Rank::Jev`) retrieves on the lane path, which also reaches
  conversation turns and the dates in a question; `max_chars` caps the memory
  content returned; `model` (`ReaderModel::Haiku` or `ReaderModel::Sonnet`)
  picks the reader in `Mode::Summary` or `Mode::Agentic`. None is sent unless
  set, so existing calls are unchanged.
- **Lane-path fields.** `Memory` carries `store`, `said` and `excerpted`;
  `RecallResult` carries `rank` and `rank_fallback`, and `Timings` the lane
  path's `embed_ms`, `lanes_ms`, `rank_ms` and per-lane `lane`.

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
