import sys
from pathlib import Path

import pytest

from openbrain_sdk.error import OpenBrainError
from openbrain_sdk.mcp import OpenBrainMcpClient


def test_mcp_tool_call_and_error_mapping():
    script = Path(__file__).with_name("fake_mcp_server.py")
    client = OpenBrainMcpClient.spawn_openbrain_mcp(
        openbrain_path=sys.executable,
        args=[str(script)],
    )
    try:
        pong = client.call_tool("openbrain.ping", {})
        assert pong["version"] == "0.1"

        result = client.call_tool(
            "openbrain.search.semantic",
            {"scope": "work", "query": "q", "top_k": 3},
        )
        assert result["ok"] is True

        with pytest.raises(OpenBrainError) as exc:
            client.call_tool("openbrain.search.semantic", {"scope": "missing-provider", "query": "q"})
        assert exc.value.code == "OB_NOT_FOUND"
    finally:
        client.close()
