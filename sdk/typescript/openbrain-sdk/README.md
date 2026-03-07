# OpenBrain TypeScript SDK

Node-first SDK for OpenBrain HTTP (`openbrain serve`) and MCP stdio (`openbrain mcp`) surfaces.

## Quick install

From this folder:

```bash
npm install
npm run build
```

## Usage - HTTP (mirror/debug API)

```ts
import { OpenBrainHttpClient } from "./src/http.js";

const client = new OpenBrainHttpClient({ baseUrl: "http://127.0.0.1:7981" });

await client.write({
  objects: [
    {
      type: "decision",
      id: "decision-1",
      scope: "project-alpha",
      status: "canonical",
      spec_version: "0.1",
      tags: ["example"],
      data: { decision: "Adopt OpenBrain" },
      provenance: { actor: "agent" },
    },
  ],
});

const scoped = await client.read({ scope: "project-alpha", refs: ["decision-1"] });

const semantic = await client.searchSemantic({
  scope: "project-alpha",
  query: "adoption criteria",
  top_k: 5,
  embedding_provider: "openai",
  embedding_model: "text-embedding-3-small",
  embedding_kind: "semantic",
});

console.log(semantic.matches[0]?.ref);
```

## Usage - MCP (stdio tools)

```ts
import { OpenBrainMcpClient } from "./src/mcp.js";

const client = await OpenBrainMcpClient.spawnOpenBrainMcp({
  openbrainPath: "openbrain",
});

const pong = await client.callTool("openbrain.ping", {});
console.log(pong.version);

await client.callTool("openbrain.memory.pack", {
  scope: "project-alpha",
  task_hint: "Summarize recent decisions",
});

await client.close();
```

## Error handling

Errors are normalized to `OpenBrainError` with stable fields:

- `code`
- `status` (HTTP status when applicable)
- `details`
- `message`

## Tests

```bash
npm test
```

