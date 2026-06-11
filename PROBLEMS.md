# Workflow Codebase — Design & Logic Problems

46 problems found across `src/`. Organized by priority.

---

## P0 — Memory Safety / Panics / Data Corruption

| # | File:Line | Problem | Impact | Test |
|---|-----------|---------|--------|------|
| 24 | `experience/pool.rs:162-165` | No bounds validation on mmap `count` from file header | UB on corrupted/truncated mmap file | `test_corrupted_header_count_causes_ub` |
| 30 | `l0/mod.rs:53` + `resource.rs:68-76` | Depth increment TOCTOU race (load-check-add not atomic) | Spawn depth limit violated under concurrency | `test_depth_increment_toctou_race` |
| 38 | `experience/dual_track.rs:273` | `consolidate()` drains all fluid, drops data if no cluster qualifies | Permanent data loss of all fluid experiences | `test_consolidate_drops_data_when_no_cluster_qualifies` |
| 20 | `l2/llm.rs:102` | Indexes `contending_agents[0]` without bounds check | Panic on adversarial/corrupted LLM output | `test_empty_contending_agents_panics` |
| 46 | `l2/llm.rs:97` | Winner parsed as `usize` but prompt says agent_id (`[u8; 16]`) | Override arbitration almost always picks first agent | `test_parse_decision_override_with_agent_id` |
| 45 | `l2/llm.rs:187-188` | Override patch uses first embedding, not winner's | Wrong agent's vector stored in experience pool | `test_override_patch_uses_first_embedding` |
| 28 | `experience/clustering.rs:66,72` | Division by zero `new_weight` when all entries have weight 0 | NaN contaminates all clusters | — |
| 37 | `l2/mod.rs:36` | `contending_agents[0]` without empty check | Panic on malformed conflict manifest | — |

## P1 — Data Loss / Broken Logic

| # | File:Line | Problem | Impact | Test |
|---|-----------|---------|--------|------|
| 1 | `tools/builtin.rs:69-75` | ReadFile uses line numbers as byte offsets in `content[start..end]` | Panics or wrong data on multi-byte UTF-8 | `test_read_file_multibyte_panics` |
| 14 | `persistence.rs:222-224` | `remove_file` before `rename` in `write_atomic` | Crash between = data loss; `rename` alone is atomic | `test_write_atomic_dangerous_remove_before_rename` |
| 11 | `provider.rs:64-65` | `last_used()` always returns ~0ns (creates Instant then reads elapsed) | Health tracking completely broken | — |
| 12 | `provider.rs:199-201` | Unhealthy clients never evicted from pool | Memory leak — broken clients accumulate forever | — |
| 13 | `provider.rs:149-151` | `mark_success()` called on cache hit, not actual API call | Failed clients look healthy after lookup | — |
| 41 | `runtime/mod.rs:357` | `_value_statement` param ignored, embeds `"default"` literal | L1/L2 value evaluation deaf to caller intent | — |
| 42 | `runtime/mod.rs:622` | `parent_span_id` hardcoded 0 in `spawn_child` | Trace lineage broken for all child agents | — |
| 44 | `l2/llm.rs:89-92` | Medium risk resets failure counter to 0 | Collapse protection bypassed by alternating medium-risk | `test_medium_risk_resets_failure_counter` |
| 34 | `agent/plan.rs:517-519` | `execute_plan` returns `Ok(())` even on task failure | Plan failures silently swallowed | — |
| 33 | `config.rs:237-241` | `merge_configs` last-source-wins, doc says first-source-wins | Wrong config when 3+ sources collide | — |
| 43 | `experience/clustering.rs:72` | Welford M2 uses wrong weight factor | Cluster variance systematically underestimated | — |

## P2 — Concurrency / Race Conditions

| # | File:Line | Problem | Impact | Test |
|---|-----------|---------|--------|------|
| 30 | `l0/mod.rs:53` | Depth increment TOCTOU race | Spawn depth limit violated (also in P0) | `test_depth_increment_toctou_race` |
| 25 | `suspend.rs:58-59` | NaN priority maps to `Equal` via `unwrap_or` | Unpredictable sort position for NaN entries | — |
| 3 | `tools/builtin.rs:192` | Blocking `std::process::Command` in async fn | Stalls tokio runtime under concurrent calls | — |
| 36 | `config.rs:184-187` | Blocking `TcpStream::connect_timeout` in async startup | Stalls TUI for seconds with 6+ providers | — |

## P3 — UI / Rendering Bugs

| # | File:Line | Problem | Impact | Test |
|---|-----------|---------|--------|------|
| 14 | `tui/chat.rs:54-55` | Cursor X uses full input width, not current line width | Wrong cursor position in multi-line input | — |
| 17 | `tui/chat_lines.rs:593-606` | Table headers stored as separate rows, only first rendered | Multi-column table headers silently lost | — |
| 29 | `tui/chat_lines.rs:789-805` | Byte/char index mismatch in span splitting | Garbled text for CJK/emoji | — |
| 40 | `tui/chat_lines.rs:574-608` | Table data rows merged into single row | All multi-row tables render as one garbled row | — |
| 39 | `tui/render.rs:36` | Chat cache keyed on message count, not content | Final completed response invisible until next message | — |
| 7 | `tui/handler.rs:255` | `chat_scroll = 0` on submit | Visual flicker before auto-scroll | — |
| 19 | `tui/state.rs:236-239` | Response index not adjusted after tool call insertion | Fragile if insert semantics change | — |

## P4 — UTF-8 / Multi-byte Panics

| # | File:Line | Problem | Impact | Test |
|---|-----------|---------|--------|------|
| 21 | `tui/dialogs/custom_wizard.rs:160` | `&self.url[..37]` byte-slice splits multi-byte UTF-8 | Panic on non-ASCII URLs | — |
| 22 | `agent/plan.rs:407` | Curly-quote `\u{201c}` is 3 bytes, offset advances 1 | Invalid UTF-8 slice, panic | — |
| 35 | `tui/effect.rs:326` | `&s[..57]` hardcoded byte-slice on multi-byte string | Panic on non-ASCII tool args | — |
| 27 | `tui/dialogs/key.rs:162` | Cursor uses byte `.len()` not char count | Cursor misalignment for non-ASCII API keys | — |

## P5 — Dead Code / Misleading APIs

| # | File:Line | Problem | Impact | Test |
|---|-----------|---------|--------|------|
| 4 | `l1/arbitration.rs:33-35` | Inverted priority: identical embeddings → agent B always wins | Arbitration result is arbitrary | `test_identical_embeddings_agent_b_wins`, `test_priority_inversion_higher_sim_lower_priority` |
| 6 | `runtime/pipeline.rs:219-224 vs 270-275` | `add_experience` and `record_experience` are identical | Dead duplicate methods | — |
| 10 | `core/simd.rs:19` | `cosine_similarity_768` operates on 384-dim | Misleading API name | — |
| 11 | `llm/embed.rs:50-58` | Dead `embed_768` returns `[f32; 768]` but codebase uses 384 | Dead code, type mismatch if used | — |
| 12 | `llm/mod.rs:128-131` | `tokens_used` always hardcoded to 0 | Telemetry silently lost | — |
| 5 | `l2/mod.rs:20-21` | `consecutive_failures` keeps incrementing after collapse | Counter grows unbounded | `test_consecutive_failures_increment_after_collapse` |
| 21 | `l2/llm.rs:57-58` | `consecutive_failures` wraps to 0 after ~4B increments | Silent exit from collapse state | `test_consecutive_failures_increment_after_collapse` |

## P6 — Logic / Comparison Errors

| # | File:Line | Problem | Impact | Test |
|---|-----------|---------|--------|------|
| 26 | `models.rs:335` | Priority merge uses `>=` instead of `>` | Equal-priority providers clobber each other | — |
| 8 | `experience/clustering.rs:66-72` | Double-normalization in Welford variance | Underestimated cluster variance | — |
| 6 | `experience/dual_track.rs:77` | `entries.remove(0)` on Vec is O(n) | Performance: should use VecDeque | — |
| 10 | `l2/mod.rs:119` | Trait method `audit` shadows inherent method `audit` | Confusing dispatch, fragile | — |
| 20 | `agent/agent.rs:152-154` | `agent_id_str` only formats 4 of 16 bytes | Agent ID collisions break lookup | — |
| 22 | `tools/builtin.rs:143-144` | WriteFile no bounds check on `start+200` | Panic if content is short | `test_write_file_short_content_panics` |

---

## Summary

| Priority | Count | Category |
|----------|-------|----------|
| P0 | 8 | Memory safety, panics, data corruption |
| P1 | 11 | Data loss, broken logic |
| P2 | 4 | Concurrency, blocking in async |
| P3 | 7 | UI/rendering bugs |
| P4 | 4 | UTF-8 multi-byte panics |
| P5 | 7 | Dead code, misleading APIs |
| P6 | 6 | Logic/comparison errors |
| **Total** | **46** | |

## Test Coverage

Tests added to confirm bugs:

| Test | File | Bug # | What it confirms |
|------|------|-------|------------------|
| `test_read_file_multibyte_panics` | `tools/builtin.rs` | #1 | ReadFile panics on multi-byte UTF-8 |
| `test_write_file_short_content_panics` | `tools/builtin.rs` | #22 | WriteFile panics with short content |
| `test_identical_embeddings_agent_b_wins` | `l1/arbitration.rs` | #4 | Agent B always wins with identical embeddings |
| `test_priority_inversion_higher_sim_lower_priority` | `l1/arbitration.rs` | #4 | Less similar agent wins due to priority inversion |
| `test_empty_contending_agents_panics` | `l2/llm.rs` | #20 | Panic on empty contending_agents |
| `test_medium_risk_resets_failure_counter` | `l2/llm.rs` | #44 | Medium risk resets failure counter to 0 |
| `test_override_patch_uses_first_embedding` | `l2/llm.rs` | #45 | Override patch uses first embedding, not winner's |
| `test_consecutive_failures_increment_after_collapse` | `l2/llm.rs` | #5,#21 | Counter keeps incrementing during collapse |
| `test_parse_decision_override_with_agent_id` | `l2/llm.rs` | #46 | Winner parsed as usize but prompt says agent_id |
| `test_depth_increment_toctou_race` | `l0/resource.rs` | #30 | TOCTOU race in depth increment |
| `test_consolidate_drops_data_when_no_cluster_qualifies` | `experience/dual_track.rs` | #38 | Consolidation permanently loses all fluid data |
| `test_corrupted_header_count_causes_ub` | `experience/pool.rs` | #24 | Corrupted mmap header count causes UB |
| `test_write_atomic_dangerous_remove_before_rename` | `persistence.rs` | #14 | write_atomic uses dangerous remove+rename pattern |
