import assert from "node:assert/strict";
import http from "node:http";
import { once } from "node:events";
import { describe, it } from "node:test";
import { OpenBrainHttpClient } from "../src/http.js";
import { OpenBrainError } from "../src/error.js";
import { ReadRequest } from "../src/types.js";

async function runMockServer(
  onRequest: (req: { method: string; path: string; body: Record<string, unknown> }) => unknown
): Promise<{ baseUrl: string; close: () => Promise<void> }> {
  const server = http.createServer(async (req, res) => {
    const chunks: Buffer[] = [];
    for await (const chunk of req) {
      chunks.push(Buffer.from(chunk));
    }
    const body = chunks.length ? JSON.parse(Buffer.concat(chunks).toString("utf8")) : {};
    const response = onRequest({
      method: req.method ?? "",
      path: req.url ?? "",
      body: body as Record<string, unknown>,
    });

    res.statusCode = 200;
    res.setHeader("content-type", "application/json");
    res.end(JSON.stringify(response));
  });

  await once(server.listen(0), "listening");
  const address = server.address();
  if (!address || typeof address === "string") {
    throw new Error("failed to start mock server");
  }
  return {
    baseUrl: `http://127.0.0.1:${address.port}`,
    close: () => new Promise((resolve, reject) => server.close((err) => (err ? reject(err) : resolve()))),
  };
}

describe("OpenBrain HTTP client", () => {
  it("routes semantic search with embedding selectors", async () => {
    let seenBody: Record<string, unknown> | undefined;
    const server = await runMockServer((req) => {
      seenBody = req.body;
      if (req.path === "/v1/search/semantic") {
        return {
          ok: true,
          matches: [{ ref: "r1", kind: "claim", score: 0.42, updated_at: "2026-01-01T00:00:00Z" }],
        };
      }
      return { ok: false, error: { code: "OB_NOT_FOUND", message: "unknown path" } };
    });

    const client = new OpenBrainHttpClient({ baseUrl: server.baseUrl });
    const response = await client.searchSemantic({
      scope: "work",
      query: "test",
      embedding_provider: "openai",
      embedding_model: "text-embedding-3-small",
      embedding_kind: "semantic",
      top_k: 3,
    });

    assert.equal(response.matches.length, 1);
    assert.equal(seenBody?.embedding_provider, "openai");
    assert.equal(seenBody?.embedding_model, "text-embedding-3-small");
    assert.equal(seenBody?.embedding_kind, "semantic");
    await server.close();
  });

  it("maps server error response into OpenBrainError", async () => {
    const server = await runMockServer(() => ({
      ok: false,
      error: { code: "OB_NOT_FOUND", message: "ref missing", details: { missing_refs: ["r-missing"] } },
    }));
    const client = new OpenBrainHttpClient({ baseUrl: server.baseUrl });
    const req: ReadRequest = { scope: "work", refs: ["r-missing"] };

    try {
      await client.read(req);
      assert.fail("expected throw");
    } catch (error) {
      assert.ok(error instanceof OpenBrainError);
      assert.equal((error as OpenBrainError).code, "OB_NOT_FOUND");
      assert.equal((error as OpenBrainError).details?.missing_refs[0], "r-missing");
    } finally {
      await server.close();
    }
  });
});
