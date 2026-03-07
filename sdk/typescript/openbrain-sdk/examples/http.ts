import { OpenBrainHttpClient } from "../src/http.js";

async function main() {
  const client = new OpenBrainHttpClient();

  await client.write({
    objects: [
      {
        type: "decision",
        id: "decision-001",
        scope: "project-alpha",
        status: "canonical",
        spec_version: "0.1",
        tags: ["it", "example"],
        data: { title: "Adopt OpenBrain SDKs", body: "Ship SDKs for HTTP + MCP." },
        provenance: { actor: "example-script", method: "http-example" },
      },
    ],
  });

  await client.read({ scope: "project-alpha", refs: ["decision-001"] });

  const semantic = await client.searchSemantic({
    scope: "project-alpha",
    query: "adoption criteria",
    top_k: 5,
    embedding_provider: "openai",
    embedding_model: "text-embedding-3-small",
    embedding_kind: "semantic",
  });

  console.log("matches:", semantic.matches.length);

  await client.memoryPack({
    scope: "project-alpha",
    task_hint: "Summarize current decisions",
  });
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});

