from openbrain_sdk import OpenBrainMcpClient


def main() -> None:
    client = OpenBrainMcpClient.spawn_openbrain_mcp(openbrain_path="openbrain")
    try:
        pong = client.call_tool("openbrain.ping", {})
        print(pong["version"])
        pack = client.call_tool(
            "openbrain.memory.pack",
            {"scope": "project-alpha", "task_hint": "Summarize recent decisions"},
        )
        print(pack["pack"]["summary"])
    finally:
        client.close()


if __name__ == "__main__":
    main()

