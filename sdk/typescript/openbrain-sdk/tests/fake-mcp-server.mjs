import readline from "node:readline";

function rpcResponse(id, result, error) {
  if (error) {
    return { jsonrpc: "2.0", id, error };
  }
  return { jsonrpc: "2.0", id, result };
}

function envelopeOk(payload) {
  return { ok: true, ...payload };
}

function envelopeErr(code, message, details) {
  return { ok: false, error: { code, message, details } };
}

const rl = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });

rl.on("line", (line) => {
  let req;
  try {
    req = JSON.parse(line);
  } catch (_err) {
    return;
  }

  if (req.method === "tools/call") {
    const { name, arguments: args = {} } = req.params || {};
    if (!name) {
      console.log(JSON.stringify(rpcResponse(req.id, envelopeErr("OB_INVALID_REQUEST", "missing tool name"))));
      return;
    }

    if (name === "openbrain.ping") {
      console.log(
        JSON.stringify(rpcResponse(req.id, envelopeOk({ version: "0.1", server_time: "now" })))
      );
      return;
    }

    if (name === "openbrain.write") {
      const objects = Array.isArray(args?.objects) ? args.objects.length : 0;
      console.log(
        JSON.stringify(
          rpcResponse(
            req.id,
            envelopeOk({
              results: [{ ref: "r1", type: "claim", status: "draft", version: 1, ...(objects ? {} : {}) }],
            })
          )
        )
      );
      return;
    }

    if (name === "openbrain.search.semantic") {
      if (!args || typeof args.query !== "string") {
        console.log(
          JSON.stringify(
            rpcResponse(req.id, envelopeErr("OB_INVALID_REQUEST", "invalid search.semantic request", { missing: "query" }))
          )
        );
        return;
      }

      if (args.scope === "missing-provider") {
        console.log(
          JSON.stringify(rpcResponse(req.id, envelopeErr("OB_NOT_FOUND", "no embeddings found for provider"))
          )
        );
        return;
      }

      console.log(
        JSON.stringify(
          rpcResponse(req.id, envelopeOk({ matches: [{ ref: "r1", kind: "claim", score: 0.9, updated_at: "2026-01-01T00:00:00Z" }] }))
        )
      );
      return;
    }

    if (name === "openbrain.rerank") {
      console.log(JSON.stringify(rpcResponse(req.id, envelopeOk({ ranked_refs: ["r1"], rationale_short: [] }))));
      return;
    }

    if (name === "openbrain.memory.pack") {
      console.log(
        JSON.stringify(
          rpcResponse(req.id, envelopeOk({ pack: { scope: args?.scope || "", canonical: [], constraints: [], relevant: [], conflicts: [], summary: "ok" } }))
        )
      );
      return;
    }
  }

  console.log(
    JSON.stringify(rpcResponse(req.id, envelopeErr("OB_INVALID_REQUEST", `unknown tool: ${req?.params?.name || req?.method}`)))
  );
});
