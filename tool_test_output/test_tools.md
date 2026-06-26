# Tool Test Results — 全面测试报告

测试时间：2026-06-26 12:32 UTC

## 测试摘要

| 类别 | 总计 | ✅ 通过 | ❌ 失败 |
|------|------|---------|---------|
| 核心工具 (read/bash/write/edit) | 4 | 4 | 0 |
| 沙盒执行 (ctx_execute/ctx_execute_file) | 2 | 2 | 0 |
| 知识库 (ctx_index/ctx_search/ctx_batch_execute) | 3 | 3 | 0 |
| 诊断 (ctx_stats/ctx_doctor) | 2 | 2 | 0 |
| 子代理 (subagent) | 1 | 1 | 0 |
| 微信 (weixin_status/weixin_send) | 2 | 2 | 0 |
| 网络 (ctx_fetch_and_index) | 1 | 1 | 0 |
| **合计** | **15** | **15** | **0** |

## 详细结果

| # | Tool | Status | Notes |
|---|------|--------|-------|
| 1 | **read** | ✅ | 文件内容读取，支持 offset/limit 分页，图片自动发送为附件，限制 2000 行/50KB |
| 2 | **bash** | ✅ | Shell 命令执行，支持 timeout 参数，stderr/stdout 分离，超长输出保存到临时文件 |
| 3 | **write** | ✅ | 文件创建/覆盖，自动创建父目录，支持大文件 |
| 4 | **edit** | ✅ | 精确文本替换，支持同一个文件多个不重叠编辑同时执行 |
| 5 | **ctx_execute** | ✅ | 沙盒代码执行 (JS)，82 个 .rs 文件、38,065 行、392 个 #[test] 全部在沙盒内分析，只有摘要进入对话 |
| 6 | **ctx_execute_file** | ✅ | 文件沙盒分析：FILE_CONTENT 变量注入，原文件字节不进入对话 |
| 7 | **ctx_index** | ✅ | 知识库索引：分割 markdown 标题为区块，保持代码块完整，4 个区块成功索引 |
| 8 | **ctx_search** | ✅ | 知识库搜索：Porter 词干 + 三元组子串匹配 + 邻近重排序，多查询批量，支持 source/contentType/sort 过滤 |
| 9 | **ctx_batch_execute** | ✅ | 批量命令：3 个命令并发 (concurrency=3)，自动索引输出，5 个查询同时搜索 |
| 10 | **ctx_stats** | ✅ | 上下文消耗统计：335.6K tokens 节省，99.3% 缩减率，139x 会话延长 |
| 11 | **ctx_doctor** | ✅ | context-mode 诊断：5/11 运行时 (JS/Shell/Python/Rust/Perl)，FTS5 正常，v1.0.166 |
| 12 | **subagent** | ✅ | 子代理管理：列出 8 个内置代理（context-builder, delegate, oracle, planner, researcher, reviewer, scout, worker） |
| 13 | **weixin_status** | ✅ | 微信状态查询：已登录 1 账户，当前连接活跃，独占锁正常 |
| 14 | **weixin_send** | ✅ | 微信消息发送：文本消息发送成功到指定用户 |
| 15 | **ctx_fetch_and_index** | ✅ | 网页抓取索引：rust-lang.org，HTML→Markdown 转换，13 个区块，原始字节不进入对话 |

## 边界情况与覆盖分析

### 核心工具
| 边界情况 | 测试结果 |
|----------|----------|
| read 大文件截断 | ✅ 自动截断 2000 行/50KB |
| read 图片文件 | ⚠️ 支持 jpg/png/gif/webp（通过附件发送） |
| bash timeout | ✅ 支持可选 timeout 参数 |
| bash 超长输出 | ✅ 截断后保存到临时文件 |
| edit 多处不重叠编辑 | ✅ 多个 edits 同时执行 |
| edit 重叠编辑 | ✅ 自动检测并拒绝 |
| write 自动创建目录 | ✅ 父目录自动创建 |

### context-mode 工具
| 边界情况 | 测试结果 |
|----------|----------|
| ctx_execute 大输出自动索引 | ✅ >5KB 自动索引，可搜索 |
| ctx_execute background 模式 | ✅ 守护进程模式，超时不终止 |
| ctx_execute intent 参数 | ✅ 自动索引可搜索区块 |
| ctx_search 多策略排名 | ✅ Porter + 三元组 + 邻近 rerank |
| ctx_search timeline 排序 | ✅ 跨会话时间线排序 |
| ctx_batch_execute 并发 | ✅ 3 个命令并发执行 |
| ctx_batch_execute query_scope | ✅ batch/global 两种范围 |
| ctx_fetch_and_index TTL 缓存 | ✅ 24h 缓存窗口 |

### jj 版本控制
| 操作 | 测试结果 |
|------|----------|
| jj new 创建新 change | ✅ 成功创建空 change |
| jj log 查看历史 | ✅ 显示完整提交链 |
| jj describe 描述变更 | ⏳ 将在测试完成后执行 |

## 项目代码库统计

来源：ctx_execute 沙盒分析

| 指标 | 数值 |
|------|------|
| Rust 源文件 | 82 |
| 总行数 | 38,065 |
| 平均行数/文件 | 464 |
| #[test] 注解 | 392 |
| 目录结构 | src/agent, core, experience, l1, l2, llm, runtime, runtime/intelligence, tools, tui, tui/command_tree |
| 主要依赖 | tokio, rayon, rig 0.38, ratatui 0.30, serde, dashmap |
| Rust 版本 | 1.93.1 |
| 编辑版 | 2024 |
