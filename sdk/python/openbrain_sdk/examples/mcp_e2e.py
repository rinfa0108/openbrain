import os
import time

from openbrain_sdk import OpenBrainMcpClient


def main() -> None:
    scope = "sdk-usage-proof"
    obj_id = f"artifact-{int(time.time())}"

    env = dict(os.environ)
    env["OPENBRAIN_EMBED_PROVIDER"] = "fake"

    client = OpenBrainMcpClient.spawn_openbrain_mcp(openbrain_path="openbrain", env=env)
    try:
        ping = client.call_tool("openbrain.ping", {})
        print(f"version: {ping.get('version')}")

        write = client.call_tool(
            "openbrain.write",
            {
                "objects": [
                    {
                        "type": "artifact",
                        "id": obj_id,
                        "scope": scope,
                        "status": "canonical",
                        "spec_version": "0.1",
                        "tags": ["sdk", "usage-proof"],
                        "data": {"summary": "MCP usage proof"},
                        "provenance": {"actor": "sdk-usage-proof", "method": "mcp-e2e"},
                    }
                ]
            },
        )

        results = write.get("results", [])
        ref = results[0]["ref"] if results else obj_id
        read = client.call_tool("openbrain.read", {"scope": scope, "refs": [ref]})
        print(f"read objects: {len(read.get('objects', []))}")

        embed = client.call_tool(
            "openbrain.embed.generate",
            {"scope": scope, "target": {"ref": ref}, "model": "default"},
        )
        print(f"embedding reused: {embed.get('reused')}")

        semantic = client.call_tool(
            "openbrain.search.semantic",
            {
                "scope": scope,
                "query": "usage proof",
                "top_k": 5,
                "embedding_provider": "fake",
                "embedding_model": "default",
                "embedding_kind": "semantic",
            },
        )
        print(f"semantic matches: {len(semantic.get('matches', []))}")

        try:
            pack = client.call_tool(
                "openbrain.memory.pack",
                {"scope": scope, "task_hint": "Summarize current decisions"},
            )
            print(f"pack summary: {pack['pack']['summary']}")
        except Exception as exc:
            print(f"memory.pack skipped: {exc}")
    finally:
        client.close()


if __name__ == "__main__":
    main()
