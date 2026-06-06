# Workflow Repository

## Overview
A Rust implementation of a holographic self-evolving multi-agent system with layered decision architecture (L-1/L0/L1/L2), dynamic experience pool, and conflict arbitration.

## Key Commands
- `cargo check`: Verify compilation
- `cargo build`: Build the project
- `cargo test`: Run tests
- `cargo fmt --check`: Check formatting
- `cargo clippy`: Run linter
- `cargo run -- --tui`: Launch TUI dashboard (requires interactive terminal)

## Architecture

### Decision Pipeline
```
SpawnRequest → L-1 (Admission) → L0 (Hard Logic + Physical Arbitration) 
             → L1 (Local Reasoning + Cognitive Arbitration) 
             → L2 (Remote Audit + Final Arbitration) → SpawnDecision
```

### Core Modules
- `types.rs`: Core data structures (TaskId, AgentId, SpawnRequest, ExperienceEntry)
- `conflict.rs`: Conflict types and arbitration results
- `resource.rs`: TaskResourceState and BudgetGuard (RAII)
- `admission.rs`: L-1 semaphore-based admission control
- `l0.rs`: L0 circuit breaker (CAS budget, depth check, tool lock)
- `suspend.rs`: SuspendQueue with priority ordering
- `simd.rs`: SIMD-optimized cosine similarity
- `l1.rs`: L1 experience retrieval and value classifier
- `l1_arbitration.rs`: L1 cognitive arbitration
- `l2.rs`: L2 rule-based audit engine with collapse detection
- `l2_llm.rs`: L2 LLM-powered audit engine with judge personas
- `llm.rs`: LLM trait abstraction using rig (OpenAI/Anthropic providers)
- `embedding.rs`: Embedding service with caching and normalization
- `models.rs`: Model registry with models.dev/api.json integration
- `runtime.rs`: Agent runtime wiring full pipeline
- `tui.rs`: Terminal UI dashboard with ratatui

### Key Data Structures
- `SpawnRequest`: Task/role/value embeddings (768-dim), budget, depth
- `ExperienceEntry`: Embedding, applicability vector, tool bitmap, weight
- `BudgetGuard`: RAII resource guard with `settle(actual)` and auto-rollback
- `ConflictManifest`: Conflict type, contending agents, context embeddings

## Tech Stack
- Runtime: tokio + rayon
- LLM Framework: rig (OpenAI/Anthropic providers)
- Embeddings: OpenAI text-embedding-ada-002 via rig
- Vector Index: Flat partition + SIMD (AVX2+FMA)
- Persistence: mmap (memmap2) + Arc delayed reclamation
- Clustering: Threshold-based Leader Clustering (Welford update)
- TUI: ratatui + crossterm
- Model Registry: models.dev/api.json

## Environment Variables
- `OPENAI_API_KEY`: OpenAI API key
- `OPENAI_BASE_URL`: Custom OpenAI-compatible endpoint
- `OPENAI_MODEL`: Model to use (default: gpt-4)
- `ANTHROPIC_API_KEY`: Anthropic API key
- `ANTHROPIC_BASE_URL`: Custom Anthropic endpoint
- `ANTHROPIC_MODEL`: Model to use (default: claude-sonnet-4-20250514)

## TUI Controls

### Requirements
- Interactive terminal (TUI will fail with "No such device" in non-interactive environments)
- Network access for model registry (models.dev/api.json)

### Chat Panel
- `Tab` or `2`: Switch to Models panel
- `1`: Switch to Chat panel
- `j/k`: Scroll chat messages
- `Enter`: Submit task
- `Esc`: Clear input
- `Ctrl+C`: Quit

### Models Panel
- `/`: Enter search mode
- `j/k`: Navigate models
- `Enter`: Select model and return to Chat
- `Tab` or `Esc`: Return to Chat

### Search Mode
- Type to filter models by name, provider, or family
- `Enter`: Confirm search
- `Esc`: Cancel search
- `Backspace`: Delete character

## Testing Strategy
- L0: 100-thread concurrent CAS, zero budget/tool lock leakage
- L1: Fixed experience set recall ≥ 99%, SIMD vs scalar error < 1e-5
- L2: 50 adversarial samples, approval rate < 15%, repair coverage > 90%
- Conflicts: Simulate resource/semantic/value conflicts, verify determinism

## Key Conventions
- Default stance: "Presumed guilty" - requests rejected unless sufficient evidence
- All parameters dynamic at runtime (no static config)
- Human only validates final output (acceptance/rejection)
- Experience-driven learning with credibility weighting
- Defense in depth: L0 physical immunity → L1 cognitive defense → L2 value audit
