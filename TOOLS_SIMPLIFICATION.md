# Tool System Simplification Analysis

GVSD 协议要求：先建立全局模型，再逐层分析。

## 1. 全局模型 — 当前复杂度全景

```
                        ┌── 25 个 Tool 实现 ──┐
                        │                      │
   ┌──────────────────┐ │  ┌────────────────┐  │
   │  builtin.rs      │ │  │  17 impl Tool   │  │
   │  2704 行          │ │  │                │  │
   │                   │ │  │ read_file      │  │
   │  agent.rs         │ │  │ write_file     │  │
   │  520 行           │ │  │ sh             │  │
   │                   │ │  │ list_dir       │  │
   │  memo.rs          │ │  │ grep           │  │
   │  584 行           │ │  │ find_files     │  │
   │                   │ │  │ glob           │  │
   │  diff_edit.rs     │ │  │ move_file      │  │
   │  704 行           │ │  │ copy_file      │  │
   │                   │ │  │ delete_file    │  │
   │  structured.rs    │ │  │ append_file    │  │
   │  473 行           │ │  │ patch_file     │  │
   │                   │ │  │ line_edit      │  │
   │  sandbox.rs       │ │  │ fetch          │  │
   │  556 行           │ │  │ search_asset   │  │
   │                   │ │  │ extract_json   │  │
   │  ─────────────    │ │  │ diff_edit      │  │
   │  mod.rs 1269 行   │ │  └────────────────┘  │
   │  注册层 4 工厂函数  │ │                      │
   └──────────────────┘  └──────────────────────┘
                                ↓
                        4 工具域 + 沙箱
                        6 源文件 / 6810 行
                        + 注册标志位
                        + 工具位图 (手工维护)
```

## 2. 复杂度分析 — 每个热点

### 🔴 P0 热点: `patch_file` vs `diff_edit` 功能重叠

| 维度 | `patch_file` | `diff_edit` |
|------|-------------|-------------|
| 匹配 | 纯字符串子串 | SEARCH 块 (含上下文) |
| 唯一性 | 用 count 限制 | 上下文自动保证 |
| 批量修改 | 一次一个文本 | 多 hunk 原子 |
| LLM 训练 | 通用格式 | 原生格式 |
| 代码行 | ~130 行 | ~220 行核心逻辑 |
| **调用困惑** | **哪个更好？** | **LLM 难以选择** |

**结论：LLM 在两种工具间选择时会产生困惑。** 对于同一个任务（"修改文件中的某段代码"），两个工具都能完成，但 `diff_edit` 更可靠。`patch_file` 可以退役。

### 🟡 P1 热点: `glob` vs `find_files` 语义模糊

| 工具 | 功能 | LLM 区别难度 |
|------|------|-------------|
| `glob` | `glob "src/**/*.rs"` → 匹配路径 | 中等 |
| `find_files` | `find_files "*.rs" root:"src"` → 递归搜索 | 中等 |

两者都依赖 `walkdir + glob::Pattern`，底层逻辑几乎相同。差异仅在参数字段命名。

**结论：合并为一个 `glob` 工具，添加 `root` + `max_results` 参数。**

### 🟡 P2 热点: `line_edit` 过于复杂

- 1474 行（builtin.rs 中第二大工具）
- 4 种操作: `insert_after`, `insert_before`, `replace_range`, `delete_range`
- 自定义 diff 生成器
- 原子写入 + 回滚
- **极少 LLM 使用** — 大多数选择 `patch_file` 或 `diff_edit`

**结论：可退役。`diff_edit` + `write_file` 覆盖所有场景。**

### 🟢 P3 热点: `append_file` 薄封装

- 本质是 `write_file` + OpenOptions::append
- shell `>>` 也能完成

**结论：可退役。或合并到 `write_file`（加 mode 参数）。**

### 🟢 P3 热点: `search_asset` 条件注册

- 只在 sandbox + embedder 存在时有效
- 导致注册层增加 `with_search_asset` 标志位
- 非沙箱环境看到但用不了

**结论：保持条件注册，但简化标志位传递。**

### 🟢 P3 热点: 注册层复杂度

```
4 个工厂函数 + 1 个标志位：

create_tool_server()
  → register_sandboxed_tools(None, false)  ← 硬编码 false

create_agent_tool_server(state)
  → register_sandboxed_tools(None, false)  ← 硬编码 false
  → agent::register_tools()
  → memo::register_memo_tools()

create_sandboxed_agent_tool_server(state, sb)
  → register_sandboxed_tools(sb, sb.is_some())  ← 条件标志位
  → agent::register_tools()
  → memo::register_memo_tools()

create_tool_server_with(extra)
  → register_sandboxed_tools(None, false)  ← 硬编码 false
```

**每个新工具可能需要修改 2-3 个位置（定义、注册、位图）。**

## 3. 简化方案

### 方案 A: 退役重叠工具 (-5 个工具, -3233 行)

| 退役 | 原因 | 替代 |
|------|------|------|
| `patch_file` | `diff_edit` 完全替代 | `diff_edit` |
| `line_edit` | 过于复杂，LLM 极少用 | `diff_edit` + `write_file` |
| `glob` | 合并到 `find_files` | `find_files` (增强) |
| `append_file` | 薄封装 | `write_file` (加 append 模式) |
| `move_file` | 简化到 `copy_file` + shell | `copy_file` + `sh` |

**效果：17 内置工具 → 12 内置工具**

### 方案 B: 简化注册层 (-1 工厂函数, -200 行)

合并为两个入口：
1. `create_tool_server()` — 所有内置工具（无状态依赖）
2. `create_agent_tool_server(state, sandbox?)` — 所有工具 + 可选的沙箱

消除 `create_sandboxed_agent_tool_server()` 和 `create_tool_server_with()`。

### 方案 C: 简化工具位图

从手工 match 切换到自动推导或 proc macro：

```rust
// 现在:
pub(crate) fn tool_bit(name: &str) -> u64 {
    match name {
        "read_file" => 1 << 0,
        "write_file" => 1 << 1,
        // ... 需要手工维护
    }
}

// 改为: 编译时宏或集中定义
tool_bit_map! {
    ReadFile,       // bit 0
    WriteFile,      // bit 1
    Shell,          // bit 2
    Grep,           // bit 3
    // ...
}
```

## 4. 建议执行方案

建议分步执行：

**Step 1 (立即):** 退役 `patch_file`，提示用户改用 `diff_edit`
**Step 2 (立即):** 合并 `glob` + `find_files`
**Step 3 (可选):** 退役 `line_edit`、`append_file`
**Step 4 (可选):** 简化注册层
**Step 5 (长期):** 工具位图自动化

## 5. 影响分析

| 简化 | 风险 | 回滚成本 |
|------|------|---------|
| 退役 `patch_file` | 低 — diff_edit 严格更强 | 中 — 存储的经验引用旧工具 |
| 合 `glob`→`find_files` | 低 — 参数兼容 | 低 |
| 退役 `line_edit` | 中 — 有测试依赖 | 中 |
| 简化注册层 | 低 — 内部重构 | 低 |
| 位图自动化 | 低 — 纯编译时 | 低 |
