# Architecture

## Dependency Rule

Dependencies flow inward. No module depends on something at its own level or above.

```
tui  →  controller  →  runtime  →  pipeline  →  layer/
  ↘                    ↓                        ↓
   controller  →  runtime  →  llm           agent
                    ↓                        ↓
               models / persistence      core
```

**core/ has zero internal dependencies** — no imports from `crate::layer/`, `crate::agent/`, etc.

## Module Map

### core/ — Foundation (zero internal deps)

```
core/
├── mod.rs
├── error.rs       # SpawnRejection (thiserror)
├── simd.rs        # Cosine similarity (no deps)
└── types.rs       # SpawnRequest, SpawnDecision, ExperienceEntry,
                   # AgentId, TaskId, TraceId, and named constants
```

Only types that ALL layers share go here. Each layer owns its own types otherwise.

### layer/ — Decision pipeline layers (self-contained, depend only on core)

```
layer/
├── mod.rs
├── admission/           # L-1
│   ├── mod.rs           # AdmissionControl trait + AdmissionController
│   └── types.rs         # AdmissionPermit
├── circuit_breaker/     # L0
│   ├── mod.rs           # CircuitBreaker trait + L0CircuitBreaker
│   ├── resource.rs      # TaskResourceState + BudgetGuard
│   └── types.rs         # L0Permit
├── retrieval/           # L1
│   ├── mod.rs           # ExperienceRetrieval trait + L1Retriever
│   ├── classifier.rs    # L1ValueClassifier
│   └── arbitration.rs   # L1Arbitrator + L1ArbitrationResult
└── audit/               # L2
    ├── mod.rs           # AuditEngine trait + L2RuleAuditEngine
    ├── llm.rs           # L2LlmAuditEngine
    └── types.rs         # L2AuditResult + OverridePatch + ConflictManifest
```

Each sub-directory is **self-contained**: defines its own trait, struct, impl, and result types.
Only imports from `crate::core`.

### llm/ — LLM abstraction (depends only on core)

```
llm/
├── mod.rs               # LlmProvider enum + EmbeddingService trait
├── types.rs             # LlmRequest, LlmResponse, Message
├── chat.rs              # chat impl (build_chat_agent! macro)
├── embed.rs             # embed_768 impl
├── factory.rs           # from_env / from_key
└── embedding.rs         # EmbeddingService struct (DashMap cache)
```

### agent/ — Agent model (depends on core + llm)

```
agent/
├── mod.rs
├── agent.rs             # Agent, AgentPool, AgentConfig, AgentStatus
├── suspend.rs           # SuspendQueue + SuspendConfig
└── plan.rs              # Plan, PlanRegistry, Task
```

### pipeline/ — Decision pipeline (composes layers)

```
pipeline/
├── mod.rs               # DecisionPipeline (struct + process_request)
└── builder.rs           # DecisionPipelineBuilder (DI container)
```

Owns the `L-1 → L0 → L1 → L2` orchestration.
Takes `Box<dyn Trait>` for each layer. No concrete layer types.

### runtime/ — Runtime orchestration (thin)

```
runtime/
├── mod.rs               # AgentRuntime (config + pipeline + executor)
└── exec.rs              # AgentExecutor (spawn_root, spawn_child, execute, await)
```

AgentRuntime is just wiring:
- Holds `DecisionPipeline` + `AgentExecutor` + config + role templates
- Delegates pipeline decisions to `pipeline/`
- Delegates agent lifecycle to `exec/`

### tui/ — Terminal UI (depends on controller)

```
tui/
├── mod.rs               # Tui struct + event loop + Drop
├── handler.rs           # Keyboard event dispatch (no business logic)
├── render.rs            # draw() + layout
├── sidebar.rs           # Sidebar rendering
├── chat_lines.rs        # Chat message line building
├── dialogs/mod.rs       # Dialog rendering
├── state.rs             # AppState
├── keymap.rs            # Key bindings
└── controller.rs        # Business calls (persistence, models, runtime)
```

### Top-level

```
src/
├── main.rs              # Entry: TUI default, --cli for CLI
├── lib.rs               # Module registration (no glob re-exports)
├── models.rs            # ModelRegistry (models.dev)
└── persistence.rs       # ~/.workflow/state.json save/load
```

## Data Flow

```
User Input
  → tui/handler.rs: keystroke → UI state change + controller call
    → controller.rs: submit_chat()
      → runtime/exec.rs: chat_with_goal()
        → runtime/exec.rs: spawn_root_agent()
          → pipeline/: process_request(SpawnRequest)
            → layer/admission: acquire()
            → layer/circuit_breaker: try_acquire()
            → layer/retrieval: check_confidence()
          → Approve / Reject
        → runtime/exec.rs: execute_agent()
          → llm: chat()  (for LLM response)
          → parse_role_assignments()
          → runtime/exec.rs: spawn_child() (recursive)
```

## Key Design Decisions

1. **core/ has zero dependencies** — prevents circular imports, clean compilation
2. **Each layer owns its types** — no shared types file that everything imports
3. **L2AuditResult and OverridePatch in layer/audit/** — used by L2, not by core
4. **SpawnRejection in core/error.rs** — returned by pipeline, needed everywhere
5. **SpawnRequest/SpawnDecision in core/types.rs** — the shared contract between layers
6. **ExperienceEntry in core/types.rs** — shared between L1 (reads), runtime (stores)
7. **AgentId/TaskId in core/types.rs** — fundamental identifiers
