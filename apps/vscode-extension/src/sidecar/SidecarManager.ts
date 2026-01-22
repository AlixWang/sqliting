import * as path from "path";
import { spawn } from "child_process";

import * as vscode from "vscode";

import { NdjsonClient } from "./NdjsonClient";

export type SidecarHandle = {
  client: NdjsonClient;
  stop: () => Promise<void>;
};

export class SidecarManager {
  private readonly context: vscode.ExtensionContext;
  private readonly output: vscode.OutputChannel;
  private handle: SidecarHandle | undefined;
  private startPromise: Promise<SidecarHandle> | undefined;
  private stopRequested = false;
  private crashCount = 0;
  private lastStartAt = 0;

  constructor(context: vscode.ExtensionContext, output: vscode.OutputChannel) {
    this.context = context;
    this.output = output;
  }

  async ensureRunning(): Promise<SidecarHandle> {
    if (this.handle) return this.handle;
    if (!this.startPromise) {
      this.startPromise = this.start().finally(() => {
        this.startPromise = undefined;
      });
    }
    this.handle = await this.startPromise;
    return this.handle;
  }

  async stop(): Promise<void> {
    if (!this.handle) return;
    this.stopRequested = true;
    await this.handle.stop();
    this.handle = undefined;
  }

  private async start(): Promise<SidecarHandle> {
    this.stopRequested = false;
    const cfg = vscode.workspace.getConfiguration("sqliting");
    const overridePath = (cfg.get<string>("sidecar.path") ?? "").trim();
    const logLevel = cfg.get<string>("sidecar.logLevel") ?? "info";
    const timeoutMs = cfg.get<number>("protocol.timeoutMs") ?? 30000;

    const binPath = overridePath || this.resolveBundledSidecarPath();

    this.output.appendLine(`[sidecar] spawn: ${binPath}`);
    const now = Date.now();
    if (now - this.lastStartAt < 10_000) this.crashCount += 1;
    else this.crashCount = 0;
    this.lastStartAt = now;

    const proc = spawn(binPath, ["--log-level", logLevel], {
      cwd: this.context.extensionPath,
      stdio: ["pipe", "pipe", "pipe"],
      windowsHide: true
    });

    proc.stderr.setEncoding("utf8");
    proc.stderr.on("data", (chunk: string) => {
      for (const line of chunk.split(/\r?\n/)) {
        if (!line) continue;
        this.output.appendLine(`[sidecar] ${line}`);
      }
    });

    proc.on("error", (err) => {
      this.output.appendLine(`[sidecar] process error: ${String(err)}`);
    });

    const client = new NdjsonClient(proc, timeoutMs);

    proc.on("exit", (code, signal) => {
      this.output.appendLine(`[sidecar] exited code=${code} signal=${signal}`);
      this.handle = undefined;
      if (this.stopRequested) return;

      // Exponential backoff restart, stop after too many rapid crashes.
      if (this.crashCount >= 3) {
        this.output.appendLine("[sidecar] too many crashes; auto-restart disabled");
        return;
      }
      const delay = Math.min(30_000, 1000 * Math.pow(2, this.crashCount));
      this.output.appendLine(`[sidecar] restarting in ${delay}ms...`);
      setTimeout(() => {
        // Best-effort restart; next ensureRunning will also start if needed.
        void this.ensureRunning().catch((e) => {
          this.output.appendLine(`[sidecar] restart failed: ${String(e)}`);
        });
      }, delay);
    });

    return {
      client,
      stop: async () => {
        try {
          proc.kill();
        } catch {
          // ignore
        }
      }
    };
  }

  private resolveBundledSidecarPath(): string {
    const platform = process.platform;
    const arch = process.arch;

    // Align with RFC-003: bin/<platform> layout inside extension root.
    // For now implement the primary targets we plan to ship.
    let folder: string;
    if (platform === "win32") folder = "win32-x64";
    else if (platform === "darwin" && arch === "arm64") folder = "darwin-arm64";
    else if (platform === "darwin") folder = "darwin-x64";
    else folder = "linux-x64";

    const exe = platform === "win32" ? "sqlite-helper.exe" : "sqlite-helper";
    return path.join(this.context.extensionPath, "bin", folder, exe);
  }
}

