import * as vscode from "vscode";

import type { QueryResult } from "../protocol";

export class ResultView {
  private panel: any | undefined;
  private readonly ctx: any;
  private onMessage: ((msg: unknown) => void) | undefined;

  constructor(ctx: any) {
    this.ctx = ctx;
  }

  setMessageHandler(handler: (msg: unknown) => void) {
    this.onMessage = handler;
  }

  showQuery(sql: string, dbPath: string, result: QueryResult) {
    const panel = this.ensurePanel();
    panel.title = "SQLiting Result";
    panel.webview.html = renderHtml(sql, dbPath, result);
    panel.reveal(panel.viewColumn ?? vscode.ViewColumn.Beside, true);
  }

  private ensurePanel(): any {
    if (this.panel) return this.panel;
    this.panel = vscode.window.createWebviewPanel(
      "sqliting.result",
      "SQLiting Result",
      vscode.ViewColumn.Beside,
      { enableScripts: true, retainContextWhenHidden: true }
    );

    this.panel.webview.onDidReceiveMessage((msg: unknown) => {
      this.onMessage?.(msg);
    });

    this.panel.onDidDispose(() => {
      this.panel = undefined;
    });
    return this.panel;
  }
}

function renderHtml(sql: string, dbPath: string, result: QueryResult): string {
  const cols = result.columns ?? [];
  const rows = result.rows ?? [];
  const truncated = Boolean(result.truncated);

  const thead = cols
    .map((c) => `<th>${escapeHtml(c.name)}</th>`)
    .join("");

  const tbody = rows
    .map((r, idx) => {
      const tds = cols
        .map((c) => `<td>${escapeHtml(formatCell((r as any)[c.name]))}</td>`)
        .join("");
      return `<tr data-row="${idx}">${tds}</tr>`;
    })
    .join("");

  const banner = truncated
    ? `<div class="banner">Truncated result. Showing first ${rows.length} rows.</div>`
    : "";

  return `<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width,initial-scale=1" />
    <style>
      body { font-family: -apple-system,BlinkMacSystemFont,Segoe UI,Helvetica,Arial,sans-serif; padding: 12px; }
      .meta { color: #666; font-size: 12px; margin-bottom: 8px; }
      .sql { white-space: pre-wrap; font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace; background:#f6f8fa; border:1px solid #e1e4e8; padding:8px; border-radius:6px; }
      .toolbar { display:flex; gap:8px; margin: 10px 0; flex-wrap: wrap; }
      button { padding: 6px 10px; border: 1px solid #ccc; background: #fff; border-radius: 6px; cursor: pointer; }
      button:hover { background:#f6f8fa; }
      .banner { margin: 10px 0; padding: 8px; border-left: 4px solid #c77d00; background: #fff7e6; }
      table { border-collapse: collapse; width: 100%; }
      th, td { border: 1px solid #ddd; padding: 6px 8px; vertical-align: top; }
      th { background: #fafafa; position: sticky; top: 0; z-index: 1; }
      td { max-width: 480px; overflow-wrap: anywhere; }
      tr.selected { outline: 2px solid #4f8cc9; outline-offset: -2px; }
    </style>
  </head>
  <body>
    <div class="meta"><b>DB:</b> ${escapeHtml(dbPath)}</div>
    <div class="meta"><b>Rows:</b> ${rows.length} &nbsp; <b>Cols:</b> ${cols.length}</div>
    <div class="sql">${escapeHtml(sql)}</div>
    <div class="toolbar">
      <button id="copySql">Copy SQL</button>
      <button id="copyJson">Copy JSON</button>
      <button id="copyRowJson">Copy Selected Row (JSON)</button>
      <button id="exportCsv">Export CSV</button>
    </div>
    ${banner}
    <table>
      <thead><tr>${thead}</tr></thead>
      <tbody>${tbody}</tbody>
    </table>
    <script>
      const vscode = acquireVsCodeApi();
      let selectedRow = null;

      const tbody = document.querySelector('tbody');
      if (tbody) {
        tbody.addEventListener('click', (e) => {
          const tr = e.target && e.target.closest ? e.target.closest('tr[data-row]') : null;
          if (!tr) return;
          const idx = Number(tr.getAttribute('data-row'));
          if (!Number.isFinite(idx)) return;
          selectedRow = idx;
          for (const r of tbody.querySelectorAll('tr')) r.classList.remove('selected');
          tr.classList.add('selected');
        });
      }

      document.getElementById('copySql').addEventListener('click', () => vscode.postMessage({ type: 'copySql' }));
      document.getElementById('copyJson').addEventListener('click', () => vscode.postMessage({ type: 'copyJson' }));
      document.getElementById('exportCsv').addEventListener('click', () => vscode.postMessage({ type: 'exportCsv' }));
      document.getElementById('copyRowJson').addEventListener('click', () => {
        vscode.postMessage({ type: 'copyRowJson', rowIndex: selectedRow });
      });
    </script>
  </body>
</html>`;
}

function formatCell(v: unknown): string {
  if (v === null || v === undefined) return "";
  if (typeof v === "string") return v;
  if (typeof v === "number" || typeof v === "boolean") return String(v);
  if (typeof v === "object") {
    const o = v as any;
    if (o && o.$type === "blob") {
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

function escapeHtml(s: string): string {
  return s
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

