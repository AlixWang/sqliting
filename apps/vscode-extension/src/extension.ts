import * as vscode from "vscode";

import type { ExecResult, QueryResult } from "./protocol";
import { SidecarManager } from "./sidecar/SidecarManager";
import { SqlitingExplorer } from "./explorer/SqlitingExplorer";
import { ResultView } from "./result/ResultView";

let sidecar: SidecarManager | undefined;
let output: vscode.OutputChannel | undefined;
let explorer: SqlitingExplorer | undefined;
let resultView: ResultView | undefined;

let activeDbPath: string | undefined;
let lastQuery:
  | { sql: string; dbPath: string; result: QueryResult }
  | undefined;

export function activate(context: vscode.ExtensionContext) {
  output = vscode.window.createOutputChannel("SQLiting: Sidecar");
  sidecar = new SidecarManager(context, output);
  resultView = new ResultView(context);
  explorer = new SqlitingExplorer(context, sidecar);

  const tv = vscode.window.createTreeView("sqliting.explorer", { treeDataProvider: explorer });
  context.subscriptions.push(tv);

  context.subscriptions.push(output);

  resultView.setMessageHandler(async (msg) => {
    if (!msg || typeof msg !== "object") return;
    if (!lastQuery) {
      vscode.window.showWarningMessage("No result to operate on yet.");
      return;
    }

    if (msg.type === "copySql") {
      await vscode.env.clipboard.writeText(lastQuery.sql);
      vscode.window.showInformationMessage("SQL copied.");
      return;
    }

    if (msg.type === "copyJson") {
      await vscode.env.clipboard.writeText(JSON.stringify(lastQuery.result, null, 2));
      vscode.window.showInformationMessage("Result JSON copied.");
      return;
    }

    if (msg.type === "copyRowJson") {
      const idx = typeof msg.rowIndex === "number" ? msg.rowIndex : null;
      if (idx === null || idx < 0 || idx >= lastQuery.result.rows.length) {
        vscode.window.showWarningMessage("Select a row first.");
        return;
      }
      await vscode.env.clipboard.writeText(JSON.stringify(lastQuery.result.rows[idx], null, 2));
      vscode.window.showInformationMessage("Row JSON copied.");
      return;
    }

    if (msg.type === "exportCsv") {
      await exportLastResultCsv(context, lastQuery);
      return;
    }
  });

  context.subscriptions.push(
    vscode.commands.registerCommand("sqliting.showSidecarLogs", () => {
      output?.show(true);
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("sqliting.refreshSchema", async () => {
      explorer?.refresh();
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("sqliting.setActiveDb", async (dbPath: string) => {
      const h = await sidecar!.ensureRunning();
      await h.client.request("connect", { path: dbPath });
      activeDbPath = dbPath;
      explorer?.addRecentDb(dbPath);
      vscode.window.showInformationMessage(`SQLiting active: ${dbPath}`);
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("sqliting.previewTable", async (dbPath: string, table: string) => {
      const h = await sidecar!.ensureRunning();
      activeDbPath = dbPath;
      explorer?.addRecentDb(dbPath);
      const maxRows = vscode.workspace.getConfiguration("sqliting").get<number>("query.maxRows") ?? 1000;
      const sql = `SELECT * FROM ${table} LIMIT ${Math.min(50, maxRows)}`;
      const res = await h.client.request<{ sql: string; path?: string; limit?: number }, QueryResult>("query", {
        sql,
        path: dbPath,
        limit: Math.min(50, maxRows)
      });
      lastQuery = { sql, dbPath, result: res };
      resultView?.showQuery(sql, dbPath, res);
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("sqliting.copyTableName", async (node: any) => {
      const table = node?.table;
      if (typeof table !== "string" || !table) return;
      await vscode.env.clipboard.writeText(table);
      vscode.window.showInformationMessage("Table name copied.");
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("sqliting.insertSelectTop50", async (node: any) => {
      const table = node?.table;
      if (typeof table !== "string" || !table) return;
      const editor = vscode.window.activeTextEditor;
      if (!editor) {
        vscode.window.showWarningMessage("Open an editor to insert SQL.");
        return;
      }
      const sql = `SELECT * FROM ${table} LIMIT 50;`;
      await editor.edit((b) => b.insert(editor.selection.active, sql));
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("sqliting.exportLastResultCsv", async () => {
      if (!lastQuery) {
        vscode.window.showWarningMessage("No result to export yet.");
        return;
      }
      await exportLastResultCsv(context, lastQuery);
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("sqliting.copyLastResultJson", async () => {
      if (!lastQuery) {
        vscode.window.showWarningMessage("No result to copy yet.");
        return;
      }
      await vscode.env.clipboard.writeText(JSON.stringify(lastQuery.result, null, 2));
      vscode.window.showInformationMessage("Result JSON copied.");
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("sqliting.openDatabase", async () => {
      const pick = await vscode.window.showOpenDialog({
        canSelectMany: false,
        openLabel: "Open SQLite DB"
      });
      if (!pick?.length) return;
      const p = pick[0].fsPath;
      const h = await sidecar!.ensureRunning();
      await h.client.request("connect", { path: p });
      activeDbPath = p;
      explorer?.addRecentDb(p);
      vscode.window.showInformationMessage(`SQLiting connected: ${p}`);
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("sqliting.runQuery", async () => {
      const h = await sidecar!.ensureRunning();
      const sql = await getSqlFromEditorOrPrompt();
      if (!sql) return;
      const dbPath = await ensureDbPath();
      if (!dbPath) return;

      const maxRows = vscode.workspace.getConfiguration("sqliting").get<number>("query.maxRows") ?? 1000;
      const res = await h.client.request<{
        sql: string;
        path?: string;
        limit?: number;
      }, QueryResult>("query", { sql, path: dbPath, limit: maxRows });

      lastQuery = { sql, dbPath, result: res };
      resultView?.showQuery(sql, dbPath, res);
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("sqliting.execute", async () => {
      const cfg = vscode.workspace.getConfiguration("sqliting");
      const confirm = cfg.get<boolean>("confirmWrites") ?? true;

      const h = await sidecar!.ensureRunning();
      const sql = await getSqlFromEditorOrPrompt();
      if (!sql) return;
      const dbPath = await ensureDbPath();
      if (!dbPath) return;

      if (confirm) {
        const choice = await vscode.window.showWarningMessage(
          "Execute a write query? This will modify your database.",
          { modal: true, detail: sql },
          "Execute"
        );
        if (choice !== "Execute") return;
      }

      const res = await h.client.request<{ sql: string; path?: string }, ExecResult>("execute", {
        sql,
        path: dbPath
      });

      output?.appendLine(`[execute] ${sql}`);
      output?.appendLine(JSON.stringify(res, null, 2));
      output?.show(true);
      vscode.window.showInformationMessage(`SQLiting execute ok (changes=${res.changes})`);
    })
  );
}

export async function deactivate() {
  await sidecar?.stop();
}

async function getSqlFromEditorOrPrompt(): Promise<string | undefined> {
  const editor = vscode.window.activeTextEditor;
  const sel = editor?.selection;
  const selected = editor && sel && !sel.isEmpty ? editor.document.getText(sel) : undefined;

  if (selected && selected.trim()) return selected.trim();

  return (await vscode.window.showInputBox({
    title: "SQLiting",
    prompt: "Enter SQL to run"
  }))?.trim();
}

async function ensureDbPath(): Promise<string | undefined> {
  if (activeDbPath) return activeDbPath;
  const choice = await vscode.window.showWarningMessage(
    "No active database. Open a database first.",
    "Open Database..."
  );
  if (choice) {
    await vscode.commands.executeCommand("sqliting.openDatabase");
  }
  return activeDbPath;
}

async function exportLastResultCsv(
  context: vscode.ExtensionContext,
  q: { sql: string; dbPath: string; result: QueryResult }
) {
  const cols = q.result.columns?.map((c) => c.name) ?? [];
  const rows = q.result.rows ?? [];

  const uri = await vscode.window.showSaveDialog({
    saveLabel: "Export CSV",
    filters: { CSV: ["csv"] }
  });
  if (!uri) return;

  const lines: string[] = [];
  lines.push(cols.map(csvCell).join(","));
  for (const r of rows) {
    lines.push(cols.map((c) => csvCell(formatCsvValue((r as any)[c]))).join(","));
  }
  const content = lines.join("\n");
  await vscode.workspace.fs.writeFile(uri, Buffer.from(content, "utf8"));
  vscode.window.showInformationMessage(`Exported CSV: ${uri.fsPath}`);
}

function csvCell(v: string): string {
  // RFC4180-ish: quote if contains comma/quote/newline.
  if (/[",\r\n]/.test(v)) {
    return `"${v.replaceAll('"', '""')}"`;
  }
  return v;
}

function formatCsvValue(v: unknown): string {
  if (v === null || v === undefined) return "";
  if (typeof v === "string") return v;
  if (typeof v === "number" || typeof v === "boolean") return String(v);
  if (typeof v === "object") {
    const o = v as any;
    if (o && o.$type === "blob") {
      // don't dump base64 into CSV by default
      const size = typeof o.size === "number" ? o.size : "";
      return `[blob ${size} bytes]`;
    }
    try {
      return JSON.stringify(v);
    } catch {
      return "[object]";
    }
  }
  return String(v);
}

