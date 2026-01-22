import * as vscode from "vscode";

import type { ColumnMeta } from "../protocol";
import type { SidecarManager } from "../sidecar/SidecarManager";

type Node =
  | { kind: "root" }
  | { kind: "db"; dbPath: string; label: string }
  | { kind: "tables"; dbPath: string }
  | { kind: "table"; dbPath: string; table: string }
  | { kind: "columns"; dbPath: string; table: string }
  | { kind: "column"; dbPath: string; table: string; column: ColumnMeta };

export class SqlitingExplorer implements vscode.TreeDataProvider<Node> {
  private readonly ctx: vscode.ExtensionContext;
  private readonly sidecar: SidecarManager;
  private readonly _onDidChangeTreeData = new vscode.EventEmitter<Node | void>();
  readonly onDidChangeTreeData = this._onDidChangeTreeData.event;

  constructor(ctx: vscode.ExtensionContext, sidecar: SidecarManager) {
    this.ctx = ctx;
    this.sidecar = sidecar;
  }

  refresh() {
    this._onDidChangeTreeData.fire();
  }

  getTreeItem(element: Node): vscode.TreeItem {
    switch (element.kind) {
      case "root": {
        const item = new vscode.TreeItem("Connections", vscode.TreeItemCollapsibleState.Expanded);
        return item;
      }
      case "db": {
        const item = new vscode.TreeItem(element.label, vscode.TreeItemCollapsibleState.Expanded);
        item.description = element.dbPath;
        item.contextValue = "sqliting.db";
        item.command = {
          command: "sqliting.setActiveDb",
          title: "Set Active DB",
          arguments: [element.dbPath]
        };
        return item;
      }
      case "tables": {
        return new vscode.TreeItem("Tables", vscode.TreeItemCollapsibleState.Expanded);
      }
      case "table": {
        const item = new vscode.TreeItem(element.table, vscode.TreeItemCollapsibleState.Collapsed);
        item.contextValue = "sqliting.table";
        item.command = {
          command: "sqliting.previewTable",
          title: "Preview Table",
          arguments: [element.dbPath, element.table]
        };
        return item;
      }
      case "columns": {
        return new vscode.TreeItem("Columns", vscode.TreeItemCollapsibleState.Expanded);
      }
      case "column": {
        const label = element.column.name;
        const item = new vscode.TreeItem(label, vscode.TreeItemCollapsibleState.None);
        const typ = element.column.decl_type ?? element.column.sqlite_type ?? "";
        if (typ) item.description = typ;
        item.contextValue = "sqliting.column";
        return item;
      }
    }
  }

  async getChildren(element?: Node): Promise<Node[]> {
    if (!element) return [{ kind: "root" }];

    if (element.kind === "root") {
      const dbs = this.getRecentDbs();
      return dbs.map((p) => ({ kind: "db", dbPath: p, label: basename(p) }));
    }

    if (element.kind === "db") {
      return [{ kind: "tables", dbPath: element.dbPath }];
    }

    if (element.kind === "tables") {
      const h = await this.sidecar.ensureRunning();
      const tables = await h.client.request<{ path?: string }, string[]>("tables", { path: element.dbPath });
      return tables.map((t) => ({ kind: "table", dbPath: element.dbPath, table: t }));
    }

    if (element.kind === "table") {
      return [{ kind: "columns", dbPath: element.dbPath, table: element.table }];
    }

    if (element.kind === "columns") {
      const h = await this.sidecar.ensureRunning();
      const cols = await h.client.request<{ table: string; path?: string }, ColumnMeta[]>("columns", {
        table: element.table,
        path: element.dbPath
      });
      return cols.map((c) => ({ kind: "column", dbPath: element.dbPath, table: element.table, column: c }));
    }

    return [];
  }

  addRecentDb(dbPath: string) {
    const key = "sqliting.recentDbs";
    const cur = this.ctx.workspaceState.get<string[]>(key, []);
    const next = [dbPath, ...cur.filter((x) => x !== dbPath)].slice(0, 10);
    void this.ctx.workspaceState.update(key, next);
    this.refresh();
  }

  private getRecentDbs(): string[] {
    return this.ctx.workspaceState.get<string[]>("sqliting.recentDbs", []);
  }
}

function basename(p: string): string {
  const parts = p.split(/[\\/]/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : p;
}

