# RFC-002: Rust Sidecar (`sqlite-helper`) Detailed Design

* **Version:** v1.0
* **Status:** Draft
* **Component:** Rust Sidecar (`sqlite-helper`)
* **Depends on:** `RFC-001-Architecture.md`

## 1. 背景与目标

`RFC-001` 定义了 `sqlite-helper` 的统一架构：同一个 Rust 二进制，按 CLI 参数切换两种上层通信模式（VS Code Bridge / MCP Server），共享同一套 Core 领域逻辑与 SQLite 基础设施。

本 RFC 细化 **Rust 侧**的可实现设计：模块拆分、并发模型、协议实现、类型映射、错误模型、安全边界与测试/发布策略。

### 1.1 Goals

- **统一核心逻辑**：连接管理、SQL 执行、Schema 获取、类型映射在两个模式共享。
- **稳定协议**：NDJSON（Mode A）与 MCP（Mode B）各自明确消息结构、版本与扩展点。
- **读写安全**：在 MCP 模式中对写操作显式标注 Side-effect；在核心层提供只读判定能力。
- **高性能与可控资源**：大结果集分页/截断；避免一次性加载巨量数据导致 OOM。
- **可观测性**：stdout 仅用于协议输出；stderr 用于日志与诊断。

### 1.2 Non-goals

- 不在 Rust 侧实现 VS Code UI/渲染。
- 不做分布式/远程服务（仅 stdio 本地进程）。
- 不自动修改用户数据库的 PRAGMA（除非工具/命令明确要求）。
- 不在本 RFC 内冻结 MCP 规范细节（以 MCP 标准为准，本 RFC给出实现建议与兼容层）。

## 2. 二进制入口与运行模式

### 2.1 CLI 设计

基于 `clap`：

- `sqlite-helper`：默认进入 **Mode A: VS Code Bridge**
- `sqlite-helper --mcp`：进入 **Mode B: MCP Server**
- `--log-level <error|warn|info|debug|trace>`：控制 stderr 日志级别（默认 `info`）
- `--protocol-version <int>`：可选，强制协议版本（默认当前）
- `--max-rows <int>`：查询返回的最大行数上限（默认 1000；MCP 的 Resource 预览默认 50）
- `--timeout-ms <int>`：单次请求软超时（默认 30000）
- `--allowed-dir <path>`（可重复）：启用路径白名单（主要用于 MCP 模式；未设置则默认“仅允许访问请求的绝对路径”，不做目录限制）

### 2.2 I/O 约束

- **stdout**：仅输出协议消息（NDJSON 或 JSON-RPC）。
- **stderr**：日志、诊断、panic hook 输出（但实现目标是 no-panic）。
- **flush**：每条响应写入后强制 flush，避免缓冲导致 VS Code 端等待。

## 3. 模块划分与目录结构（Monorepo 统一建议）

`RFC-002` 与 `RFC-003` 将落在同一个仓库中，因此目录结构需要从“单项目”升级为“多组件同仓库”的统一布局。

### 3.1 仓库顶层结构（建议）

```text
.
├─ crates/
│  └─ sqlite-helper/                  # Rust sidecar（本 RFC 关注）
│     ├─ Cargo.toml
│     └─ src/...
├─ apps/
│  └─ vscode-extension/               # VS Code Extension（RFC-003 关注）
│     ├─ package.json
│     ├─ src/...
│     ├─ media/...                   # webview 静态资源
│     └─ bin/...                     # 内置 sidecar 二进制（按平台）
├─ packages/
│  └─ protocol/                       # 共享协议/类型（NDJSON v1 / MCP 映射）
│     ├─ ndjson/...
│     └─ mcp/...
├─ scripts/                           # 构建/打包 glue（cargo build + copy bins + vsix）
├─ rfcs/
└─ .github/workflows/
```

约定：

- **Rust** 只在 `crates/` 下；可用 Cargo workspace 管理多 crate（未来如拆 `core`/`mcp`/`bridge` 为独立 crate 也仍在 `crates/`）。
- **VS Code 扩展** 只在 `apps/vscode-extension/` 下；发布 VSIX 时以该目录作为扩展根。
- **协议共享** 放在 `packages/protocol/`：用于集中维护 `RFC-001` 中 NDJSON 与 MCP 的字段/错误码/示例，避免 Rust/TS 各写一套导致漂移。
- **构建脚本** 放 `scripts/`：统一“编 sidecar -> 拷贝到 extension/bin -> 打 VSIX”的流水线（细节见 `RFC-003`）。

### 3.2 `sqlite-helper` crate 内部结构（建议）

```text
crates/sqlite-helper/
  Cargo.toml
  src/
    main.rs
    cli.rs
    error.rs
    logging.rs
    core/
      mod.rs
      connection.rs
      query.rs
      schema.rs
      types.rs
      limits.rs
      readonly.rs
    adapters/
      mod.rs
      vscode_bridge/
        mod.rs
        protocol.rs
        handler.rs
        io.rs
      mcp/
        mod.rs
        jsonrpc.rs
        handler.rs
        tools.rs
        resources.rs
        prompts.rs
    infra/
      mod.rs
      sqlite.rs
      fs.rs
    tests/
      fixtures.rs
```

- **core/**：与“上层协议无关”的领域能力。
- **adapters/**：Mode A / Mode B 的协议与路由。
- **infra/**：rusqlite、文件系统等基础设施封装。

## 4. 并发与连接管理

### 4.1 约束

- `rusqlite::Connection` **不是** `Send + Sync`；不能在多线程间直接共享。
- 但实际需求包括：UI 并发请求（比如同时加载表与预览数据）、MCP 并发工具调用。

### 4.2 推荐模型：单线程 DB Worker + 消息队列

采用 **单个 DB Worker 线程**（或每个 db_path 一个 worker），将所有 sqlite 操作串行化，保证线程安全与可预测的锁行为。

- **请求执行**：Adapter 将“数据库操作请求”发送到 worker（channel）。
- **响应返回**：worker 执行后回传结果（oneshot channel）。
- **好处**：无需引入连接池 crate；行为稳定；便于实现超时与取消。

#### 4.2.1 多数据库支持

`ConnectionManager` 维护 `HashMap<DbKey, WorkerHandle>`：

- `DbKey`：建议使用规范化后的绝对路径（Windows 需 canonicalize）
- worker 生命周期：首次 `connect`/首次访问创建；空闲 N 分钟可回收（可选）。

### 4.3 Busy/Lock 策略

- 连接创建时设置 `busy_timeout`（例如 2000ms，可配置），避免“立刻报锁”。
- 对写操作（execute / write_query）可允许更长 timeout（可配置）。

## 5. Core 领域能力

### 5.1 Query 执行（Read / Write）

核心 API（概念）：

- `open(db_path) -> DbHandle`
- `query(db, sql, limits) -> QueryResult`
- `execute(db, sql) -> ExecResult`

#### 5.1.1 只读判定（用于 MCP 的 read_query）

不要用字符串前缀简单判断；优先使用 SQLite 本身的只读判定能力：

- `conn.prepare(sql)?` 得到 `Statement`
- 使用 SQLite 的 `sqlite3_stmt_readonly` 等价能力（通过 rusqlite Statement 暴露的 readonly 判定）判断
- 若判定为非只读，则 read_query 返回错误（code: `NOT_READONLY`）

> 备注：`WITH`、`PRAGMA` 等语句的只读性由 SQLite 判定更可靠。

### 5.2 Schema 获取

实现：

- `tables`: 查询 `sqlite_master` 过滤 `type='table' AND name NOT LIKE 'sqlite_%'`
- `columns`: `PRAGMA table_info(<table>)`，并补充 `column_decltype`（若可从 rusqlite 提取）

可扩展：

- indexes: `PRAGMA index_list`, `PRAGMA index_info`
- foreign keys: `PRAGMA foreign_key_list`

### 5.3 结果结构与顺序

`RFC-001` 提到列顺序必须稳定。推荐结构：

- `columns: Vec<ColumnMeta>`（用于 UI 渲染与顺序对齐）
- `rows: Vec<DbRow>`（每行是对象：`{colName: value}`，与 `RFC-001` 一致）

建议 `ColumnMeta`：

- `name: String`
- `decl_type: Option<String>`（DDL 声明类型）
- `sqlite_type: Option<String>`（运行时推断/声明）

#### 5.3.1 QueryResult 的 JSON 形态（v1）

为与 `RFC-001` 对齐，Mode A 与 Mode B 的查询结果统一为：

```json
{
  "columns": [
    { "name": "id", "decl_type": "INTEGER", "sqlite_type": "INTEGER" },
    { "name": "name", "decl_type": "TEXT", "sqlite_type": "TEXT" }
  ],
  "rows": [
    { "id": 1, "name": "alice" },
    { "id": 2, "name": "bob" }
  ],
  "truncated": false,
  "next_offset": null
}
```

说明：

- `rows` 内对象的 key 必须是列名；列渲染顺序以 `columns` 为准。
- `decl_type/sqlite_type` 可为空（未能获取/不适用时）。
- 若发生截断：`truncated: true`，并尽量提供 `next_offset`（数字）；不支持时可省略或设为 `null`。

### 5.4 类型映射（SQLite -> JSON）

遵循 `RFC-001` 的映射，补充细节：

- `INTEGER` -> JSON number（i64）
- `REAL` -> JSON number（f64）
- `TEXT` -> JSON string
- `BLOB` -> JSON object：`{"$type":"blob","base64":"...","size":123}`
- `NULL` -> JSON null

对浮点数 NaN/Inf：

- JSON 不支持，遇到 NaN/Inf 时应返回 error（code: `INVALID_NUMBER`）或转为 string（需协议标记，建议优先报错以保持语义一致）。

### 5.5 限制与分页（大结果集）

必须提供防护：

- `max_rows`：默认 1000（Mode A）、MCP tool 默认 1000、Resource 预览默认 50
- `max_bytes`：可选，限制单次响应体积（例如 5MB）

对超限行为：

- 截断返回：`truncated: true`，并返回 `next_offset`（如果支持 offset）

建议支持 `query` payload：

- `sql: string`
- `params?: any[]`（预留参数化）
- `limit?: number`
- `offset?: number`

## 6. Mode A（VS Code Bridge）实现细化

### 6.1 传输与 framing

- stdin/stdout
- 一行一个 JSON 对象（NDJSON）
- 只接受 UTF-8
- 忽略空行

### 6.2 消息版本化

建议在请求中加入 `v` 字段，便于未来升级：

```json
{"v":1,"id":"uuid","cmd":"query","payload":{"sql":"SELECT 1"}}
```

响应也回传 `v`：

```json
{"v":1,"id":"uuid","status":"ok","data":{...}}
```

### 6.3 命令路由（与 RFC-001 对齐）

支持 `RFC-001` 的基础命令：

- `connect {path}`
- `query {sql, limit?, offset?}`
- `execute {sql}`
- `tables {path?}`（未提供则使用 active db）
- `columns {table, path?}`（未提供 path 则使用 active db）

建议补充但不强制（可作为 v1.1 扩展）：

- `close {path?}`：关闭连接/回收 worker
- `ping {}`：健康检查

### 6.4 Session 与多 DB

`RFC-001` 中 `connect` “存入 Session”。在 Rust 侧建议把“当前活动 DB”作为 adapter 级状态：

- 若 request payload 未带 `path`：使用 `active_db_path`
- 若带 `path`：使用指定 path（并可自动设为 active）

这样既兼容“先 connect 再 query”，也支持“直接 query 指定 db_path”。

### 6.5 错误响应模型

响应结构扩展（兼容 `RFC-001`）：

- `error`: string（对人可读）
- `code`: string（机器可读）
- `details?`: object（可选，包含 sqlite 错误码/扩展信息）

示例：

```json
{"v":1,"id":"...","status":"error","code":"SQL_ERROR","error":"near \"FROM\": syntax error","details":{"sqlite_code":1}}
```

## 7. Mode B（MCP Server）实现细化

### 7.1 JSON-RPC 2.0 over stdio

实现一个 JSON-RPC 路由器，处理：

- `initialize`
- `tools/list`, `tools/call`
- `resources/list`, `resources/read`
- `prompts/list`, `prompts/get`

> 具体字段以 MCP 最新规范为准；实现时建议做“宽松解析、严格输出”：解析时允许多余字段，输出保证符合 schema。

### 7.2 Tools 设计（对齐 RFC-001）

#### 7.2.1 `read_query`

- **输入**：`db_path: string`, `sql: string`, `limit?: number`, `offset?: number`
- **约束**：必须只读（使用 SQLite 判定）
- **输出**：`QueryResult`（包含 columns/rows/truncated）

#### 7.2.2 `write_query`（敏感）

- **输入**：`db_path: string`, `sql: string`
- **约束**：对外声明 sideEffect；adapter 不做“自动确认”，由 MCP Client 完成交互确认
- **输出**：`ExecResult {changes,last_insert_rowid?}`

#### 7.2.3 `get_schema`

- **输入**：`db_path: string`
- **输出**：`{tables:[{name,columns:[...]}]}` 或 `{tables:[...], views:[...], indexes:[...]}`（按实现迭代）

### 7.3 Resources 设计（对齐 RFC-001）

#### 7.3.1 URI 规范

`sqlite://{abs_path_to_db}/tables/{table_name}`

实现要求：

- 必须解析为本机绝对路径
- 必须进行路径规范化与白名单校验（若启用 allowed-dir）

#### 7.3.2 read 行为

- 默认返回前 50 行（可支持 query 参数 `limit`）
- 返回 JSON 快照（包含 columns/rows）

### 7.4 Prompts 设计（对齐 RFC-001）

提供 `analyze-db-health`：

- 执行 `PRAGMA integrity_check`
- 统计表行数/大小（可选：`pragma page_count`, `pragma page_size` 推算文件级别信息）
- 输出健康报告（结构化 JSON + 人类可读摘要）

## 8. 文件与路径安全

### 8.1 Path 规范化

- 输入 `db_path` 必须转换为绝对路径（相对路径拒绝或按当前工作目录解析后 canonicalize）
- Windows 下处理盘符大小写与 `\\?\` 前缀

### 8.2 目录白名单（可配置）

若用户通过 `--allowed-dir` 配置白名单：

- 仅允许访问白名单目录的子路径
- 否则返回 `PATH_NOT_ALLOWED`

### 8.3 SQL 注入与参数化

由于 sql 由用户/AI 提供，本质上无法“防止注入”，但可以提供：

- tool schema 强制 `sql` 为 string
- 预留 `params` 参数化接口（v1 可不实现，v1.1 支持）

## 9. 错误处理（No Panic）

### 9.1 Error enum（建议）

分类建议（code）：

- `INVALID_REQUEST`：协议层字段缺失/解析失败
- `PATH_NOT_ALLOWED`：路径越权
- `DB_OPEN_FAILED`：无法打开/不是 sqlite
- `SQL_ERROR`：语法/执行错误
- `NOT_READONLY`：read_query 被判定为写
- `TIMEOUT`：超时
- `INTERNAL`：未知错误（严控出现）

### 9.2 Panic hook

尽管目标 no-panic，仍建议设置 panic hook：

- 将 panic 信息输出到 stderr
- 尽量让进程退出为非 0（避免 silent failure）

## 10. 可观测性与诊断

- 结构化日志（建议 `tracing`，也可用 `log`）输出到 stderr
- 每个 request 打印：mode、id、cmd/method、耗时、db_path（注意脱敏可选）
- 提供 `--log-level debug` 便于联调协议

## 11. 测试策略

### 11.1 单元测试（core）

- 类型映射（含 blob/base64）
- 只读判定（SELECT vs UPDATE vs PRAGMA）
- schema 查询（tables/columns）

### 11.2 集成测试（adapters）

- NDJSON request/response golden tests
- JSON-RPC 基本方法路由（initialize/tools/list/tools/call）
- 大结果集截断行为（truncated/limit/offset）

### 11.3 兼容性测试矩阵（CI）

- Windows / macOS / Linux
- SQLite 文件锁场景（并发读写）

## 12. 发布与版本管理

- `--version` 输出二进制版本与协议版本
- Rust 侧协议版本：`PROTOCOL_VSCODE_V1`、`PROTOCOL_MCP_V1`
- GitHub Actions：交叉编译并产出三平台二进制
- 对 VS Code 扩展打包：Rust 产物进入 VSIX 的 `bin/`（见 `RFC-003`）

## 13. 兼容性与迁移

- v1 的 NDJSON 字段集固定：`v,id,cmd,payload` 与响应 `v,id,status,data,error,code,details`
- v1 向后兼容策略：
  - 新增字段：允许旧客户端忽略
  - 变更字段语义：需提升 `v` 并在 Rust 侧支持 `--protocol-version`

