# GVSD — Global Verified System Design Template

Use this template for **any non-trivial** refactoring or architectural change. Fill out all sections before writing code.

> **Hard constraints (from AGENTS.md):**
> - No local / case-by-case reasoning as final logic
> - No special-case rules or exception-based logic
> - No conclusions without full coverage analysis
> - No results without adversarial testing
> - No fixes without system-wide impact analysis

---

## 1. Global Model

*Single coherent explanation of the component and its role in the system.*

```
[Component name] is responsible for [purpose]. It fits into the system as:
  ┌─ upstream: [what calls it]
  ├─ core:     [what it does]
  └─ downstream: [what it calls]
```

## 2. Core Abstractions

*Interfaces, contracts, and coupling constraints.*

| Abstraction | Contract | Constraints |
|-------------|----------|-------------|
| Trait/type | Pre/post conditions | Coupling, ownership |

## 3. System Architecture

*Component boundaries and data flow.*

```
[Caller] ──▶ [Component] ──▶ [Dependency]
                │
                ▼
           [Side effect / output]
```

## 4. Coverage Analysis

*Enumerate the full state space.*

| State | Input | Expected output | Edge? |
|-------|-------|-----------------|-------|
| ...   | ...   | ...             | ...   |

**Edge cases identified:**
**Invalid cases identified:**
**Interaction cases identified:**

## 5. Failure Mode Testing

*Adversarial inputs and counterexamples.*

| Test | Input | Expected behavior | What it breaks |
|------|-------|-------------------|----------------|
| ...  | ...   | ...               | ...            |

## 6. Verification Results

*Invariant checks and simulated execution.*

- [ ] Invariant A holds
- [ ] Invariant B holds
- [ ] Existing tests pass
- [ ] New tests cover edge cases

## 7. Final Answer

*Only if fully verified. State the precise change plan.*
