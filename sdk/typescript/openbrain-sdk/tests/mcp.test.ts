import assert from "node:assert/strict";
import { describe, it, afterEach, beforeEach } from "node:test";
import path from "node:path";
import { OpenBrainMcpClient } from "../src/mcp.js";
import { OpenBrainError } from "../src/error.js";
import { existsSync } from "node:fs";

describe("OpenBrain MCP client", () => {
  let client: OpenBrainMcpClient | undefined;
  const localCandidate = path.join(process.cwd(), "tests", "fake-mcp-server.mjs");
  const rootCandidate = path.join(
    process.cwd(),
    "sdk",
    "typescript",
    "openbrain-sdk",
    "tests",
    "fake-mcp-server.mjs"
  );
  const scriptPath = existsSync(localCandidate)
    ? localCandidate
    : rootCandidate;

  beforeEach(async () => {
    client = await OpenBrainMcpClient.spawnOpenBrainMcp({
      openbrainPath: process.execPath,
      args: [scriptPath],
      startupTimeoutMs: 2_000,
    });
  });

  afterEach(async () => {
    await client?.close();
    client = undefined;
  });

  it("forwards tools/call and unwraps envelope", async () => {
    const response = await client!.callTool("openbrain.ping", {});
    assert.equal(response.version, "0.1");
  });

  it("maps MCP tool envelope errors into OpenBrainError", async () => {
    await assert.rejects(
      () => client!.callTool("openbrain.search.semantic", { scope: "missing-provider", query: "q" }),
      (error: unknown) => {
        return error instanceof OpenBrainError && (error as OpenBrainError).code === "OB_NOT_FOUND";
      }
    );
  });
});
