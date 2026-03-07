import { OpenBrainMcpClient } from "../src/mcp.js";

async function main() {
  const scope = "sdk-usage-proof";
  const id = `artifact-${Date.now()}`;

  const client = await OpenBrainMcpClient.spawnOpenBrainMcp({
    openbrainPath: "openbrain",
    env: { ...process.env, OPENBRAIN_EMBED_PROVIDER: "fake" },
  });

  try {
    const ping = await client.callTool("openbrain.ping", {});
    console.log("version:", ping.version);

    const write = await client.callTool("openbrain.write", {
      objects: [
        {
          type: "artifact",
          id,
          scope,
          status: "canonical",
          spec_version: "0.1",
          tags: ["sdk", "usage-proof"],
          data: { summary: "MCP usage proof" },
          provenance: { actor: "sdk-usage-proof", method: "mcp-e2e" },
        },
      ],
    });

    const ref = write.results[0]?.ref ?? id;
    const read = await client.callTool("openbrain.read", { scope, refs: [ref] });
    console.log("read objects:", read.objects.length);

    const embed = await client.callTool("openbrain.embed.generate", {
      scope,
      target: { ref },
      model: "default",
    });
    console.log("embedding reused:", embed.reused);

    const semantic = await client.callTool("openbrain.search.semantic", {
      scope,
      query: "usage proof",
      top_k: 5,
      embedding_provider: "fake",
      embedding_model: "default",
      embedding_kind: "semantic",
    });
    console.log("semantic matches:", semantic.matches.length);

    try {
      const pack = await client.callTool("openbrain.memory.pack", {
        scope,
        task_hint: "Summarize current decisions",
      });
      console.log("pack summary:", pack.pack.summary);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      console.warn("memory.pack skipped:", message);
    }
  } finally {
    await client.close();
  }
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
