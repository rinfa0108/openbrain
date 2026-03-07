import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { createInterface } from "node:readline";
import { once } from "node:events";
import { OpenBrainError } from "./error.js";

type JsonRpcId = number;
type PendingResolver = (payload: unknown) => void;
type PendingRejector = (error: OpenBrainError) => void;

interface RpcRequest {
  jsonrpc: "2.0";
  id: JsonRpcId;
  method: string;
  params?: unknown;
}

interface RpcResponse {
  jsonrpc: "2.0";
  id: JsonRpcId | null;
  result?: unknown;
  error?: {
    code: number;
    message: string;
    data?: unknown;
  };
}

interface OpenBrainMcpClientOptions {
  openbrainPath?: string;
  args?: string[];
  env?: NodeJS.ProcessEnv;
  cwd?: string;
  startupTimeoutMs?: number;
}

interface PendingCall {
  resolve: PendingResolver;
  reject: PendingRejector;
}

const DEFAULT_COMMAND = "openbrain";

export class OpenBrainMcpClient {
  private readonly proc: ChildProcessWithoutNullStreams;
  private readonly openbrainPath: string;
  private readonly startupTimeoutMs: number;
  private readonly pending = new Map<JsonRpcId, PendingCall>();
  private nextId = 1;
  private closed = false;

  constructor(proc: ChildProcessWithoutNullStreams, openbrainPath: string, startupTimeoutMs: number) {
    this.proc = proc;
    this.openbrainPath = openbrainPath;
    this.startupTimeoutMs = startupTimeoutMs;
  }

  static async spawnOpenBrainMcp(
    options: OpenBrainMcpClientOptions = {}
  ): Promise<OpenBrainMcpClient> {
    const openbrainPath = options.openbrainPath ?? DEFAULT_COMMAND;
    const proc = spawn(openbrainPath, options.args ?? [], {
      stdio: ["pipe", "pipe", "pipe"],
      env: options.env ?? process.env,
      cwd: options.cwd,
      windowsHide: true,
    });

    if (!proc.stdin || !proc.stdout) {
      throw new Error(`failed to start MCP child process: ${openbrainPath}`);
    }

    const client = new OpenBrainMcpClient(
      proc,
      openbrainPath,
      options.startupTimeoutMs ?? 5000
    );
    client.bindResponseReader();
    await client.waitForTransportReady();
    return client;
  }

  private bindResponseReader() {
    const rl = createInterface({ input: this.proc.stdout });
    rl.on("line", (line) => {
      let response: RpcResponse;
      try {
        response = JSON.parse(line) as RpcResponse;
      } catch (_err) {
        return;
      }

      if (response.id == null) {
        return;
      }

      const pending = this.pending.get(response.id);
      if (!pending) {
        return;
      }
      this.pending.delete(response.id);

      if (response.error) {
        pending.reject(
          new OpenBrainError("MCP_ERROR", response.error.message, {
            details: { code: response.error.code, data: response.error.data },
          })
        );
        return;
      }

      if (
        typeof response.result === "object" &&
        response.result !== null &&
        "ok" in (response.result as Record<string, unknown>) === false
      ) {
        pending.resolve(response.result);
        return;
      }

      const result = response.result as {
        ok?: boolean;
        error?: { code: string; message: string; details?: unknown };
      };

      if (result?.ok === false && result?.error) {
        pending.reject(
          new OpenBrainError(result.error.code, result.error.message, {
            details: result.error.details,
          })
        );
        return;
      }

      pending.resolve(response.result);
    });

    this.proc.on("exit", () => {
      for (const [, pending] of this.pending) {
        pending.reject(
          new OpenBrainError("MCP_PROCESS_EXITED", "MCP process exited before response")
        );
      }
      this.pending.clear();
    });

    this.proc.on("error", (err) => {
      for (const [, pending] of this.pending) {
        pending.reject(new OpenBrainError("MCP_PROCESS_ERROR", err.message));
      }
      this.pending.clear();
    });
  }

  private async waitForTransportReady() {
    return Promise.race([
      new Promise<void>((resolve) => {
        this.proc.once("spawn", () => resolve());
      }),
      new Promise<never>((_, reject) =>
        setTimeout(() => reject(new OpenBrainError("TIMEOUT", "MCP process failed to spawn")), this.startupTimeoutMs)
      ),
    ]);
  }

  private sendRequest(method: string, params: Record<string, unknown>): Promise<unknown> {
    if (this.closed) {
      return Promise.reject(new OpenBrainError("MCP_CLOSED", "MCP session is closed"));
    }
    const id = this.nextId++;
    const request: RpcRequest = {
      jsonrpc: "2.0",
      id,
      method,
      params,
    };

    return new Promise<unknown>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.proc.stdin.write(`${JSON.stringify(request)}\n`, "utf8", (error) => {
        if (error) {
          this.pending.delete(id);
          reject(new OpenBrainError("MCP_WRITE_ERROR", error.message));
        }
      });
    });
  }

  async callTool<TToolName extends keyof import("./types.js").OpenBrainToolResponseMap>(
    toolName: TToolName,
    args: import("./types.js").OpenBrainToolRequestMap[TToolName],
    options?: { timeoutMs?: number }
  ): Promise<import("./types.js").OpenBrainToolResponseMap[TToolName]> {
    const result = await Promise.race([
      this.sendRequest("tools/call", {
        name: toolName,
        arguments: args,
      }),
      new Promise<never>((_, reject) =>
        setTimeout(
          () => reject(new OpenBrainError("TIMEOUT", "MCP tool call timeout")),
          options?.timeoutMs ?? 10_000
        )
      ),
    ]);

    return result as import("./types.js").OpenBrainToolResponseMap[TToolName];
  }

  async close() {
    if (this.closed) return;
    this.closed = true;
    this.proc.stdin.end();
    this.proc.kill();
    await once(this.proc, "exit").catch(() => undefined);
  }

}
