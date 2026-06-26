# Tool Test Results — 全面测试报告（完整版）

测试时间：2026-06-26 12:32 UTC
第二轮：2026-06-26 12:35 UTC

## 测试摘要

| 类别 | Total | ✅ Pass | ❌ Fail |
|------|-------|---------|---------|
| 核心工具 (read/bash/write/edit) | 4 | 4 | 0 |
| 沙盒执行 (ctx_execute/ctx_execute_file) | 3 | 3 | 0 |
| 知识库 (ctx_index/ctx_search/ctx_batch_execute) | 4 | 4 | 0 |
| 诊断 (ctx_stats/ctx_doctor/ctx_upgrade) | 3 | 3 | 0 |
| 子代理 (subagent) | 1 | 1 | 0 |
| 微信 (weixin_status/weixin_send) | 2 | 2 | 0 |
| 网络 (ctx_fetch_and_index/ctx_insight) | 2 | 2 | 0 |
| **合计** | **19** | **19** | **0** |

## 详细结果

| # | 工具 | 状态 | 测试内容 |
|---|------|------|----------|
| 1 | **read** | ✅ | 文件内容读取，offset/limit 分页，图片附件 |
| 2 | **bash** | ✅ | 命令执行、exit code 报告、timeout 精确终止、stderr 捕获 |
| 3 | **write** | ✅ | 文件创建/覆盖，自动创建父目录 |
| 4 | **edit** | ✅ | 精确文本替换、多处不重叠同时编辑、非唯一文本拒绝 |
| 5 | **ctx_execute (JS)** | ✅ | JS 沙盒：82 .rs 文件 / 38K 行 / 392 #[test] |
| 6 | **ctx_execute (Python)** | ✅ | Python 沙盒：57 mod tests / 平均 6.9 测试每模块 |
| 7 | **ctx_execute (Rust)** | ✅ | Rust 沙盒：205 struct / 44 enum / 14 trait / 17 unsafe |
| 8 | **ctx_execute (Shell bg)** | ✅ | Shell background 模式：超时后进程继续保持运行 |
| 9 | **ctx_execute_file** | ✅ | FILE_CONTENT 沙盒分析，文件字节不进入对话 |
| 10 | **ctx_index (内容)** | ✅ | 知识库索引：4 区块 (tool-testing-guide) |
| 11 | **ctx_index (目录)** | ✅ | 目录递归索引：docs/ → 4 文件 62 区块 |
| 12 | **ctx_search** | ✅ | 多查询批量搜索、Porter 词干 + 三元组 + 邻近重排序 |
| 13 | **ctx_batch_execute (并发3)** | ✅ | 3 命令并发 + 自动索引 + 5 查询同时搜索 |
| 14 | **ctx_batch_execute (串行1)** | ✅ | 串行模式验证 |
| 15 | **ctx_stats** | ✅ | 335.6K tokens 节省 / 99.3% 缩减 / 139× 会话延长 |
| 16 | **ctx_doctor** | ✅ | 5/11 运行时、FTS5 正常、v1.0.166 |
| 17 | **ctx_upgrade** | ✅ | 返回升级命令（未实际执行） |
| 18 | **subagent (list)** | ✅ | 列出 8 个内置子代理 |
| 19 | **subagent (get)** | ✅ | 获取 scout/delegate/reviewer 完整配置 |
| 20 | **subagent (execute scout)** | ✅ | 实际执行：25 pub fn 在 runtime.rs / 20 文件 |
| 21 | **weixin_status** | ✅ | 1 账户在线 / 独占锁正常 |
| 22 | **weixin_send** | ✅ | 文本消息发送成功 (×2) |
| 23 | **ctx_fetch_and_index (单URL)** | ✅ | rust-lang.org → 13 区块 (3.2KB) |
| 24 | **ctx_fetch_and_index (多URL并发)** | ✅ | 2 URL并发 → 7 区块 (4.6KB) |
| 25 | **ctx_insight** | ✅ | 打开 insight 浏览器面板 |

## 代码库统计汇总

| 指标 | 数值 |
|------|------|
| Rust 源文件 | 82 |
| 总行数 | 38,065 |
| `pub struct` | 205 |
| `pub enum` | 44 |
| `pub trait` | 14 |
| `unsafe` 块 | 17 |
| 类型总数 | 263 |
| `#[test]` 注解 | 392 |
| `mod tests` 块 | 57 |
| 平均测试/模块 | 6.9 |
| 目录 | 11 个子目录 |

## 边界情况覆盖

| 情况 | 测试工具 | 结果 |
|------|----------|------|
| 非唯一文本编辑 | edit | ✅ 拒绝（32 处匹配）|
| 超时命令终止 | bash | ✅ 3s timeout 精确终止 sleep 10 |
| exit code 报告 | bash | ✅ exit 42 正确显示 |
| stderr 捕获 | bash | ✅ stderr 与 stdout 分别显示 |
| 文件字节不进入对话 | ctx_execute_file | ✅ FILE_CONTENT 沙盒隔离 |
| 进程后台化 | ctx_execute | ✅ background + 超时后仍运行 |
| 并发 vs 串行 | ctx_batch_execute | ✅ 两种模式均验证 |
| 内容 vs 目录索引 | ctx_index | ✅ 两种模式均验证 |
| 单URL vs 多URL | ctx_fetch_and_index | ✅ 并发 2 并行抓取 |
| 多语言沙盒 | ctx_execute | ✅ JS / Python / Rust / Shell |
