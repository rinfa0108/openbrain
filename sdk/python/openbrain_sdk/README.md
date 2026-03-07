# OpenBrain Python SDK

Python SDK for OpenBrain HTTP (`openbrain serve`) and MCP stdio (`openbrain mcp`) clients.

## Quickstart

```python
from openbrain_sdk import OpenBrainHttpClient
from openbrain_sdk import OpenBrainMcpClient
from openbrain_sdk.models import ReadRequest, SearchSemanticRequest, MemoryObject, PutObjectsRequest

http = OpenBrainHttpClient()
http.search_semantic(
    SearchSemanticRequest(
        scope="project-alpha",
        query="find project decisions",
        embedding_provider="openai",
        embedding_model="text-embedding-3-small",
        embedding_kind="semantic",
        top_k=5,
    )
)
```

MCP usage:

```python
from openbrain_sdk import OpenBrainMcpClient

client = OpenBrainMcpClient.spawn_openbrain_mcp(openbrain_path="openbrain")
response = client.call_tool("openbrain.ping", {})
print(response["version"])
client.close()
```

## HTTP Error mapping

All structured server errors are mapped to `OpenBrainError(code, message, status?, details?)`.

