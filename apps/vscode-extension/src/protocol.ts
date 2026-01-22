export type BridgeCmd = "connect" | "query" | "execute" | "tables" | "columns";

export interface BridgeRequest<TPayload> {
  v: 1;
  id: string;
  cmd: BridgeCmd;
  payload: TPayload;
}

export interface BridgeResponse<TData> {
  v: 1;
  id: string;
  status: "ok" | "error";
  data?: TData;
  error?: string;
  code?: string;
  details?: Record<string, unknown>;
}

export interface ColumnMeta {
  name: string;
  decl_type?: string | null;
  sqlite_type?: string | null;
}

export interface QueryResult {
  columns: ColumnMeta[];
  rows: Array<Record<string, unknown>>;
  truncated?: boolean;
  next_offset?: number | null;
}

export interface ExecResult {
  changes: number;
  last_insert_rowid?: number | null;
}

