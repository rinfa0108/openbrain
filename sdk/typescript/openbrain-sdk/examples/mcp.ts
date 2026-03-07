import { OpenBrainMcpClient } from "../src/mcp.js";

async function main() {
  const client = await OpenBrainMcpClient.spawnOpenBrainMcp({
    openbrainPath: "openbrain",
  });

  try {
    await client.callTool("openbrain.ping", {});

    await client.callTool("openbrain.write", {
      objects: [
        {
          type: "artifact",
          id: "artifact-001",
          scope: "project-alpha",
          status: "canonical",
          spec_version: "0.1",
          tags: ["example", "mcp"],
          data: { summary: "Example artifact" },
          provenance: { actor: "mcp-example", type: "agent" },
        },
      ],
    });

    const pack = await client.callTool("openbrain.memory.pack", {
      scope: "project-alpha",
      task_hint: "Summarize relevant context",
    });
    console.log("pack summary:", pack.pack.summary);
  } finally {
    await client.close();
  }
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});

