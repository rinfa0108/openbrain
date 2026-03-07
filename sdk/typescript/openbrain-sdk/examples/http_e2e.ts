import { OpenBrainHttpClient } from "../src/http.js";

async function main() {
  const client = new OpenBrainHttpClient();
  const scope = "sdk-usage-proof";
  const id = `decision-${Date.now()}`;

  await client.ping();

  const write = await client.write({
    objects: [
      {
        type: "decision",
        id,
        scope,
        status: "canonical",
        spec_version: "0.1",
        tags: ["sdk", "usage-proof"],
        data: { title: "Adopt OpenBrain SDKs", reason: "Developer ergonomics" },
        provenance: { actor: "sdk-usage-proof", method: "http-e2e" },
      },
    ],
  });

  const ref = write.results[0]?.ref ?? id;
  const read = await client.read({ scope, refs: [ref] });
  console.log("read objects:", read.objects.length);

  const embed = await client.embedGenerate({
    scope,
    target: { ref },
    model: "default",
  });
  console.log("embedding reused:", embed.reused);

  const semantic = await client.searchSemantic({
    scope,
    query: "developer ergonomics",
    top_k: 5,
    embedding_provider: "fake",
    embedding_model: "default",
    embedding_kind: "semantic",
  });
  console.log("semantic matches:", semantic.matches.length);

  try {
    const pack = await client.memoryPack({
      scope,
      task_hint: "Summarize current decisions",
    });
    console.log("pack summary:", pack.pack.summary);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    console.warn("memory.pack skipped:", message);
  }
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
