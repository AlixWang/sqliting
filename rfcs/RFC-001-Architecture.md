# RFC-001: SQLite Extension Unified Sidecar Architecture

* **Version:** v1.0 (Unified)
* **Status:** Approved
* **Component:** Rust Sidecar (`sqlite-helper`)
* **Target:** VS Code Extension & MCP Clients (Claude/Cursor/Copilot)

## 1. 概述 (Executive Summary)

本项目旨在构建一个名为 `sqlite-helper` 的高性能、双模式 Rust 二进制程序。它作为 VS Code 插件的**Sidecar (边车)** 运行，旨在解决两大核心问题：

1. **高性能 I/O**: 绕过 Node.js 原生模块的 ABI 兼容性问题，提供极速的 SQLite 读写能力。
2. **AI 赋能 (MCP)**: 实现 Model Context Protocol (MCP)，允许 AI Agent 直接理解数据库结构、编写 SQL 并分析数据。

## 2. 系统架构 (System Architecture)

### 2.1 双模网关设计 (Dual-Mode Gateway)

程序启动时通过命令行参数决定运行模式。底层核心业务逻辑（Core Logic）是共享的，仅上层通信适配器不同。

* **Mode A: VS Code Bridge (GUI 模式)**
* **触发方式**: `./sqlite-helper` (无参数)
* **通信协议**: Custom NDJSON (Newline Delimited JSON)
* **特点**: 轻量、同步/异步混合、针对 UI 渲染优化（行列数据）。
* **客户**: VS Code Plugin (TypeScript).


* **Mode B: MCP Server (AI 模式)**
* **触发方式**: `./sqlite-helper --mcp`
* **通信协议**: Standard MCP Protocol (JSON-RPC 2.0 via Stdio)
* **特点**: 语义化、Tool 使用、Resource 映射、安全沙箱。
* **客户**: Claude Desktop, Cursor, VS Code Copilot, etc.



### 2.2 模块分层 (Layering)

```text
[ External World ]      [ Adaptation Layer ]           [ Core Domain ]            [ Infrastructure ]

VS Code GUI  ----->   [ VS Code Adapter ]  \
(TypeScript)          (Custom Protocol)     \
                                             \
                                              --->  [ Connection Manager ]  --->  [ Rusqlite ]
                                             /      (Thread-safe Pool)            (Native Driver)
                                            /
AI Agents    ----->   [ MCP Adapter ]      /                                      [ File System ]
(Claude/LLM)          (MCP Protocol)      /

```

---

## 3. 技术栈与依赖 (Tech Stack)

**Cargo.toml:**

```toml
[dependencies]
# 核心数据库驱动 (静态链接，无须系统依赖)
rusqlite = { version = "0.31", features = ["bundled", "column_decltype"] }

# 序列化
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# CLI 参数解析 (用于模式切换)
clap = { version = "4.4", features = ["derive"] }

# 异步运行时 (MCP 必需)
tokio = { version = "1.0", features = ["full"] }

# 错误处理
thiserror = "1.0"

# (可选) MCP SDK，或者手撸 JSON-RPC
# mcp_server = "..." 

```

---

## 4. Mode A: VS Code Bridge 协议规范

专为插件 GUI 设计的轻量级协议。

* **传输**: Stdin / Stdout
* **格式**: 一行一个 JSON 对象。

### 4.1 请求 (TS -> Rust)

```json
{
  "id": "uuid-v4",
  "cmd": "connect | query | execute | tables | columns",
  "payload": { ... }
}

```

### 4.2 响应 (Rust -> TS)

```json
{
  "id": "uuid-v4",
  "status": "ok | error",
  "data": { ... },   // 成功时存在
  "error": "..."     // 失败时存在
}

```

### 4.3 核心命令定义

| 命令 (`cmd`) | Payload 参数 | 返回 `data` 结构 | 描述 |
| --- | --- | --- | --- |
| `connect` | `{"path": "/a.db"}` | `true` | 连接数据库，存入 Session |
| `query` | `{"sql": "SELECT..."}` | `[{"col": "val"}, ...]` | 查询数据 (Read) |
| `execute` | `{"sql": "UPDATE..."}` | `{"changes": 1}` | 执行操作 (Write) |
| `tables` | `{}` | `["users", "logs"]` | 获取表名列表 |
| `columns` | `{"table": "users"}` | `[{"name": "id", "type": "INT"}]` | 获取表结构元数据 |

---

## 5. Mode B: MCP Server 协议规范

遵循 Model Context Protocol 标准，暴露能力给 AI。

### 5.1 MCP Tools (工具暴露)

AI 可调用的函数列表。

**1. `read_query**`

* **描述**: "Execute a read-only SQL query (SELECT) to analyze data."
* **参数**: `sql` (string), `db_path` (string)
* **逻辑**: 调用 Core 层的 query 方法。

**2. `write_query**` (敏感操作)

* **描述**: "Execute INSERT/UPDATE/DELETE queries. Requires user confirmation."
* **参数**: `sql` (string), `db_path` (string)

**3. `get_schema**`

* **描述**: "Get database structure (tables and columns) to write correct SQL."
* **参数**: `db_path` (string)

### 5.2 MCP Resources (资源)

允许 AI 将数据库视为文件资源读取。

* **URI**: `sqlite://{abs_path_to_db}/tables/{table_name}`
* **行为**: 读取该 Resource 时，返回该表前 50 行数据的 JSON 快照，用于快速预览。

### 5.3 MCP Prompts (预设指令)

* **Prompt**: `analyze-db-health`
* **行为**: 自动执行 `PRAGMA integrity_check`，统计表大小，并返回一份健康报告。

---

## 6. 数据结构与类型映射 (Type Mapping)

Rust 负责将 SQLite 的动态类型转换为 JSON。

| SQLite Type | Rust (Rusqlite) | JSON Output | 备注 |
| --- | --- | --- | --- |
| `INTEGER` | `i64` | `Number` |  |
| `REAL` | `f64` | `Number` |  |
| `TEXT` | `String` | `String` |  |
| `BLOB` | `Vec<u8>` | `String` (Base64) | 需要在 JSON 中标注这是二进制 |
| `NULL` | `Option::None` | `null` |  |

**Rust Core 结构示例:**

```rust
// 统一的行数据结构
pub type DbRow = std::collections::HashMap<String, serde_json::Value>;

// 统一的返回结果
pub struct QueryResult {
    pub columns: Vec<String>, // 保证列顺序
    pub rows: Vec<DbRow>,
}

```

---

## 7. 错误处理与安全性 (Error & Security)

### 7.1 错误处理

* **No Panic**: Rust 端严禁 Panic，必须捕获所有 `Result`。
* **错误传播**: 数据库错误（如 "File locked", "Syntax error"）应转换为 JSON 错误消息返回。

### 7.2 安全性 (Mode B 特有)

* **路径白名单**: 建议限制 MCP 模式只能访问用户明确授权的目录（虽然 MCP 客户端本身也会做一层限制）。
* **写操作警告**: `write_query` 工具必须包含 Side-effect 标记，提示 LLM 这是一个修改操作。

---

## 8. 开发路线图 (Implementation Roadmap)

1. **Phase 1: 脚手架与核心 (Core)**
* 建立 Rust 项目。
* 实现 `ConnectionManager` (单例或 Map 管理多连接)。
* 实现 `query` 和 `execute` 的通用封装。


2. **Phase 2: VS Code Bridge (Mode A)**
* 实现 Stdin 循环读取。
* 实现 `clap` 参数解析。
* 完成 TS 端与 Rust 端的联调，跑通 GUI 表格显示。


3. **Phase 3: MCP Server (Mode B)**
* 引入 MCP 协议处理（或手写 JSON-RPC 2.0 路由）。
* 实现 `tools/list` 和 `tools/call`。
* 在 Claude Desktop 中配置测试。


4. **Phase 4: 发布与 CI**
* 配置 Github Actions。
* 实现跨平台交叉编译 (Win/Mac/Linux)。
* 打包 VSIX。