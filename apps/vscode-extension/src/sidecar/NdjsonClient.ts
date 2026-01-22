import { randomUUID } from "crypto";
import type { ChildProcessWithoutNullStreams } from "child_process";

import type { BridgeRequest, BridgeResponse, BridgeCmd } from "../protocol";

type Pending = {
  resolve: (value: any) => void;
  reject: (err: Error) => void;
  timer: NodeJS.Timeout;
};

export class NdjsonClient {
  private readonly proc: ChildProcessWithoutNullStreams;
  private readonly timeoutMs: number;
  private buffer = "";
  private pending = new Map<string, Pending>();

  constructor(proc: ChildProcessWithoutNullStreams, timeoutMs: number) {
    this.proc = proc;
    this.timeoutMs = timeoutMs;

    this.proc.stdout.setEncoding("utf8");
    this.proc.stdout.on("data", (chunk: string) => this.onStdout(chunk));
    this.proc.on("exit", (code, signal) => {
      const err = new Error(`sidecar exited (code=${code}, signal=${signal})`);
      for (const [, p] of this.pending) {
        clearTimeout(p.timer);
        p.reject(err);
      }
      this.pending.clear();
    });
  }

  async request<TPayload, TData>(cmd: BridgeCmd, payload: TPayload): Promise<TData> {
    const id = randomUUID();
    const req: BridgeRequest<TPayload> = { v: 1, id, cmd, payload };

    const line = JSON.stringify(req) + "\n";
    this.proc.stdin.write(line, "utf8");

    return await new Promise<TData>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`timeout waiting for response: cmd=${cmd}, id=${id}`));
      }, this.timeoutMs);

      this.pending.set(id, { resolve, reject, timer });
    });
  }

  private onStdout(chunk: string) {
    this.buffer += chunk;
    while (true) {
      const idx = this.buffer.indexOf("\n");
      if (idx === -1) break;

      const line = this.buffer.slice(0, idx).trim();
      this.buffer = this.buffer.slice(idx + 1);
      if (!line) continue;

      let msg: BridgeResponse<any>;
      try {
        msg = JSON.parse(line);
      } catch {
        // ignore invalid lines (sidecar should not output junk on stdout)
        continue;
      }

      const p = this.pending.get(msg.id);
      if (!p) continue;
      clearTimeout(p.timer);
      this.pending.delete(msg.id);

      if (msg.status === "ok") {
        p.resolve(msg.data);
      } else {
        p.reject(new Error(`${msg.code ?? "ERROR"}: ${msg.error ?? "unknown error"}`));
      }
    }
  }
}

