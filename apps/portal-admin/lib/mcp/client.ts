/**
 * mcp-server HTTP client (server-side only).
 *
 * Wraps the `tools/call` endpoint of the EFE mcp-server. Mirrors the
 * Rust efe-admin McpClient (cf. `crates/efe-admin/src/mcp/mod.rs`).
 *
 * IMPORTANT : never instantiate from a Client Component — bearer
 * tokens MUST stay server-side. Only Server Components, Server
 * Actions and tRPC procedures may use this.
 */

const DEFAULT_TIMEOUT_MS = 30_000;

export interface McpClientOptions {
  baseUrl: string;
  bearerToken?: string;
  tenant: string;
  timeoutMs?: number;
}

export class McpToolError extends Error {
  public readonly code: string;
  public readonly statusCode?: number;

  constructor(code: string, message: string, statusCode?: number) {
    super(message);
    this.name = "McpToolError";
    this.code = code;
    this.statusCode = statusCode;
  }
}

interface ToolCallResponse<T> {
  result?: T;
  error?: { code: string; message: string; status_code?: number };
}

export class McpClient {
  constructor(private readonly opts: McpClientOptions) {
    if (!opts.baseUrl) throw new Error("McpClient: baseUrl required");
    if (!opts.tenant) throw new Error("McpClient: tenant required");
  }

  /** Invoke an mcp-server tool by name with JSON-serializable args. */
  async callTool<T = unknown>(name: string, args: Record<string, unknown>): Promise<T> {
    const url = `${this.opts.baseUrl.replace(/\/$/, "")}/mcp/tools/call`;
    const controller = new AbortController();
    const timeoutId = setTimeout(
      () => controller.abort(),
      this.opts.timeoutMs ?? DEFAULT_TIMEOUT_MS,
    );

    try {
      const headers: Record<string, string> = {
        "Content-Type": "application/json",
        "X-EFE-Tenant": this.opts.tenant,
      };
      if (this.opts.bearerToken) {
        headers.Authorization = `Bearer ${this.opts.bearerToken}`;
      }
      const res = await fetch(url, {
        method: "POST",
        headers,
        body: JSON.stringify({ name, arguments: args }),
        signal: controller.signal,
        // Server-side fetch ; Next.js dedups + caches by default. Disable.
        cache: "no-store",
      });
      const envelope = (await res.json()) as ToolCallResponse<T>;
      if (envelope.error) {
        throw new McpToolError(
          envelope.error.code,
          envelope.error.message,
          envelope.error.status_code,
        );
      }
      if (!res.ok) {
        throw new McpToolError(
          `http_${res.status}`,
          `mcp-server returned ${res.status}`,
          res.status,
        );
      }
      if (envelope.result === undefined) {
        throw new McpToolError("internal", "mcp-server returned neither result nor error");
      }
      return envelope.result;
    } finally {
      clearTimeout(timeoutId);
    }
  }
}

/** Singleton-ish factory used by tRPC context. */
export function createMcpClient(tenant: string, bearerToken?: string): McpClient {
  return new McpClient({
    baseUrl: process.env.EFE_MCP_HTTP_URL ?? "http://localhost:8080",
    bearerToken,
    tenant,
  });
}
