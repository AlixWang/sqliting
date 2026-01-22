# RFC-003: VS Code Extension Detailed Design

* **Version:** v1.0
* **Status:** Draft
* **Component:** VS Code Extension (TypeScript)
* **Depends on:** `RFC-001-Architecture.md`, `RFC-002-Rust-Sidecar-Design.md`

## 1. 背景与目标

`sqlite-helper` 作为 sidecar 负责高性能 SQLite 访问与 MCP 能力；VS Code 插件负责：

- 管理 sidecar 进程生命周期（启动/重启/退出/日志）
- 实现 Mode A（NDJSON）的协议客户端（请求-响应、超时、取消、并发）
- 提供清晰的 DB 浏览、SQL 编辑与结果展示 UI
- 对写操作提供用户确认（UI 模式）与可视化提示
-（可选）为用户提供 MCP 配置入口/快捷启动（不强耦合 VS Code 内部 AI）

## 2. Extension 范围与非目标

### 2.1 Goals

- **可靠进程管理**：sidecar 崩溃/升级/路径变化可恢复。
- **协议健壮**：stdout NDJSON 行分割、处理半包/黏包、容错。
- **良好 UX**：DB 连接、schema 树、查询、结果表格、错误提示。
- **安全默认**：写操作二次确认；尽量避免误修改数据库。

### 2.2 Non-goals

- 不在插件中直接集成 SQLite Node 原生驱动（避免 ABI）。
- 不实现远程 DB 连接（仅本地 sqlite 文件）。
- 不在插件内实现完整 SQL 语言服务（语法高亮/补全可依赖现有扩展或后续规划）。

## 3. 总体架构

```text
VS Code Extension (TS)
  ├─ SidecarManager  (spawn/monitor/restart)
  ├─ NdjsonClient    (request/response, timeout, cancel)
  ├─ DbWorkspaceState (connections, active db, history)
  ├─ UI
  │   ├─ ExplorerView (TreeView: db/tables/columns)
  │   ├─ SqlEditor    (TextDocument + commands)
  │   └─ ResultView   (Webview grid / table rendering)
  └─ Commands
      openDb / runQuery / runExecute / refreshSchema / export / copy
```

### 3.1 仓库目录结构（与 RFC-002 对齐）

本扩展与 `sqlite-helper` 将位于同一个仓库，统一建议的顶层结构如下（节选）：

```text
.
├─ crates/sqlite-helper/        # Rust sidecar（RFC-002）
├─ apps/vscode-extension/       # VS Code extension（本 RFC）
│  ├─ src/...
│  ├─ media/...
│  └─ bin/...                  # 内置 sidecar（按平台）
├─ packages/protocol/           # 协议/类型共享（避免 Rust/TS 漂移）
└─ scripts/                     # 构建/打包 glue（cargo + copy + vsix）
```

说明：

- 扩展发布为 VSIX 时，`apps/vscode-extension/` 目录内容会成为 **扩展根目录**；因此 VSIX 内的 `bin/...` 对应 repo 内的 `apps/vscode-extension/bin/...`。

## 4. Sidecar 生命周期管理

### 4.1 二进制定位策略

优先级（从高到低）：

- 设置项 `sqliting.sidecar.path`：用户自定义路径
- 扩展内置 `apps/vscode-extension/bin/<platform>/sqlite-helper[.exe]`（发布到 VSIX 后在扩展根目录下即为 `bin/<platform>/...`）
-（可选）开发模式：`crates/sqlite-helper/target/release/sqlite-helper[.exe]`（本仓库内 cargo build 的产物）

平台判断：

- `win32`: `sqlite-helper.exe`
- `darwin`, `linux`: `sqlite-helper`

### 4.2 启动参数

Mode A（默认）：

- `args: []`

环境变量（可选）：

- `RUST_LOG` 或扩展自定义 `--log-level`（以 `RFC-002` 为准）

### 4.3 进程管理

`SidecarManager` 负责：

- `start(): Promise<void>`
- `stop(): Promise<void>`
- `restart(): Promise<void>`
- `ensureRunning(): Promise<void>`（惰性启动）

关键点：

- **stdout**：交给 `NdjsonClient` 做协议解析。
- **stderr**：写入 `OutputChannel`（例如 `SQLiting: Sidecar`），并在 debug 时提示用户。
- **退出处理**：进程非 0 退出时，标记不可用并触发自动重启策略（带退避）。

#### 4.3.1 自动重启与退避

建议：

- 3 次快速崩溃（例如 10s 内）后停止自动重启，提示用户查看日志/路径。
- 正常运行后崩溃：指数退避（1s/2s/4s，封顶 30s）。

#### 4.3.2 版本握手（可选但推荐）

在 sidecar 启动后发送：

- `ping`（若实现）或 `tables`（fallback）验证通路
-（推荐）新增 `version` 命令（v1.1）：返回 sidecar 版本、协议版本，便于诊断兼容性

## 5. NDJSON 协议客户端（Mode A）

### 5.1 请求/响应关联

`NdjsonClient` 内部维护：

- `pending: Map<string, {resolve, reject, timer}>`
- `id` 使用 uuid v4
- 每个请求一个超时计时器（默认 30s，可配置）

### 5.2 解析策略

必须处理：

- stdout chunk 可能不以 `\n` 结尾（半包）
- 多条消息同一个 chunk（黏包）

实现建议：

- 使用 string buffer 累积
- 按 `\n` split，最后一段留在 buffer
- 每行 `JSON.parse`，解析失败记录 stderr 日志并丢弃该行（或进入“协议错误”状态）

### 5.3 API（建议）

```ts
interface BridgeRequest<TPayload> {
  v: 1;
  id: string;
  cmd: "connect" | "query" | "execute" | "tables" | "columns";
  payload: TPayload;
}

interface BridgeResponse<TData> {
  v: 1;
  id: string;
  status: "ok" | "error";
  data?: TData;
  error?: string;
  code?: string;
  details?: Record<string, unknown>;
}
```

对外暴露方法：

- `connect(path: string): Promise<boolean>`
- `query(args: { sql: string; path?: string; limit?: number; offset?: number }): Promise<QueryResult>`
- `execute(args: { sql: string; path?: string }): Promise<ExecResult>`
- `tables(path?: string): Promise<string[]>`（未传 path 则使用 active db）
- `columns(table: string, path?: string): Promise<ColumnMeta[]>`（未传 path 则使用 active db）

其中 `QueryResult`（与 `RFC-001/RFC-002` 对齐）建议为：

```ts
interface ColumnMeta {
  name: string;
  decl_type?: string | null;
  sqlite_type?: string | null;
}

interface QueryResult {
  columns: ColumnMeta[];
  rows: Array<Record<string, unknown>>;
  truncated?: boolean;
  next_offset?: number | null;
}
```

### 5.4 取消（可选）

v1 可以不实现；v1.1 规划：

- `cancel {id}`：请求 sidecar 取消正在执行的 SQL
- VS Code 侧用 `CancellationToken` 映射到 `cancel`

## 6. 插件功能与 UI 设计

### 6.1 Explorer（TreeView）

视图层级建议：

- **Connections**
  - `<db file name>`（db_path）
    - Tables
      - `<table>`
        - Columns（展开后展示字段列表）

交互：

- 单击 DB：设为 active
- 右键 table：
  - `Preview Top 50`（生成 `SELECT * FROM table LIMIT 50`）
  - `Copy CREATE TABLE`（可选：需要 sidecar 提供 DDL 获取）

### 6.2 SQL 执行入口

建议命令：

- `SQLiting: Open Database...`
- `SQLiting: Run Query (Read)`
- `SQLiting: Execute (Write)`
- `SQLiting: Refresh Schema`
- `SQLiting: Show Sidecar Logs`

Read/Write 区分：

- `Run Query`：默认用于 SELECT/PRAGMA/EXPLAIN（读）
- `Execute`：用于 INSERT/UPDATE/DELETE/DDL（写）

### 6.3 写操作确认（强制）

在 UI 模式下，对 `execute`：

- 执行前弹窗确认（显示将要执行的 SQL，支持“再次确认”）
- 支持设置项：`sqliting.confirmWrites`（默认 true）
- 若用户选择取消：直接返回，不向 sidecar 发送

### 6.4 ResultView（结果展示）

实现方式（二选一）：

- **方案 A：Webview Grid**（推荐）
  - 优点：大表格渲染更灵活（虚拟滚动、冻结列、复制）
  - 缺点：需要 webview 与 extension 通信
- **方案 B：OutputChannel/文本**（最低可用）
  - 快速可用但体验差

建议 v1 使用 Webview，最少支持：

- columns + rows 表格
- truncated 标识（“已截断，显示前 N 行”）
- Copy cell / Copy row as JSON
- Export CSV（v1 可在 extension 端做；大数据建议后续交给 sidecar）

### 6.5 分页与截断 UX

当 sidecar 返回 `truncated: true`：

- UI 显示 “Load more” 按钮（触发 offset += currentRows）
- 或提示用户自行加 `LIMIT/OFFSET`

## 7. 状态管理与持久化

使用：

- `context.workspaceState`：保存工作区内最近打开 DB 列表、active db
- `context.globalState`：全局最近打开 DB（可选）

状态结构建议：

- `recentDbs: string[]`（去重、最多 10）
- `activeDbPath?: string`
- `lastQueryByDb: Record<string,string>`（可选）

## 8. 配置项（Settings）

建议 `package.json contributes.configuration`：

- `sqliting.sidecar.path`（string, optional）
- `sqliting.sidecar.logLevel`（enum）
- `sqliting.protocol.timeoutMs`（number, default 30000）
- `sqliting.query.maxRows`（number, default 1000）
- `sqliting.confirmWrites`（boolean, default true）

## 9. 错误处理与用户提示

### 9.1 典型错误

- sidecar 不存在/不可执行：提示配置 `sqliting.sidecar.path`
- DB 文件不存在/无权限：提示用户选择正确文件
- locked/busy：提示“数据库被占用”，建议关闭占用进程或稍后重试
- SQL 语法错误：在 UI 中高亮显示错误信息，并可复制

### 9.2 输出与诊断

- 所有 sidecar stderr 写入 `OutputChannel`
- 关键错误弹出 `window.showErrorMessage` 并提供动作：
  - `Open Logs`
  - `Retry`

## 10. MCP（Mode B）在 VS Code 侧的支持（可选）

本项目的核心是 Mode A；但为了便于用户使用 MCP，可提供：

- `SQLiting: Show MCP Setup`：展示如何在外部 MCP Client 配置 `sqlite-helper --mcp`
- `SQLiting: Start MCP Server`：以 `--mcp` 启动 sidecar，并提示“当前 VS Code 会话已启动 MCP（stdio）”，同时给出常见配置示例（需要明确：很多 MCP Client 需要自己 spawn 进程，VS Code 内启动未必可复用）

> 重要：不要假设 VS Code 内某个 AI 功能一定能直连该 MCP；以“提供配置说明/快捷命令”为主。

## 11. 测试策略

### 11.1 单元测试

- NDJSON parser：半包/黏包、多行、非法 JSON 行
- pending map：超时、并发、多请求乱序返回

### 11.2 集成测试（建议）

- 以测试 sidecar（或 mock sidecar）验证端到端：
  - spawn -> connect -> tables -> query -> execute
- Windows/Linux/macOS CI 运行（至少做 parser 与逻辑层测试）

## 12. 打包与发布（VSIX）

### 12.1 产物布局

建议 **VSIX（扩展根目录）** 内包含（相对扩展根目录路径）：

```text
extension/
  bin/
    win32-x64/sqlite-helper.exe
    darwin-x64/sqlite-helper
    darwin-arm64/sqlite-helper
    linux-x64/sqlite-helper
```

对应到 **repo 源码路径**：

- `apps/vscode-extension/bin/win32-x64/sqlite-helper.exe`
- `apps/vscode-extension/bin/darwin-x64/sqlite-helper`
- `apps/vscode-extension/bin/darwin-arm64/sqlite-helper`
- `apps/vscode-extension/bin/linux-x64/sqlite-helper`

### 12.2 安装后权限

- macOS/Linux：确保可执行位（若打包系统不保留，需要在激活时 `chmod +x`；注意 VS Code 扩展的可写目录限制）

### 12.3 版本兼容

- 插件启动时可检查 sidecar `version/protocol`（若实现），不兼容时提示用户升级扩展或 sidecar。

