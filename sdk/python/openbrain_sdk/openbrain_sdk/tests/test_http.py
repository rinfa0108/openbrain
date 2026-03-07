import json
import threading
import urllib.parse
from http.server import BaseHTTPRequestHandler, HTTPServer

import pytest

from openbrain_sdk.client import OpenBrainHttpClient
from openbrain_sdk.error import OpenBrainError
from openbrain_sdk.models import ReadRequest, SearchSemanticRequest


class MockOpenBrainHandler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length).decode("utf-8")
        payload = json.loads(body) if body else {}

        if self.path == "/v1/search/semantic":
            response = {
                "ok": True,
                "matches": [
                    {
                        "ref": "r1",
                        "kind": "claim",
                        "score": 0.7,
                        "updated_at": "2026-01-01T00:00:00Z",
                    }
                ],
            }
            self.server.last_semantic_body = payload
        elif self.path == "/v1/read":
            response = {
                "ok": False,
                "error": {
                    "code": "OB_NOT_FOUND",
                    "message": "missing refs",
                    "details": {"missing_refs": ["r-missing"]},
                },
            }
        else:
            response = {"ok": False, "error": {"code": "OB_NOT_FOUND", "message": "unknown path"}}

        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps(response).encode("utf-8"))


class MockOpenBrainServer(threading.Thread):
    def __init__(self):
        super().__init__(daemon=True)
        self.httpd: HTTPServer | None = None
        self.port = 0
        self.last_semantic_body = None

    def run(self):
        server = HTTPServer(("127.0.0.1", 0), MockOpenBrainHandler)
        self.httpd = server
        server.last_semantic_body = None
        self.port = server.server_address[1]
        server.serve_forever()

    def stop(self):
        if self.httpd:
            self.httpd.shutdown()


@pytest.fixture()
def mock_server():
    server = MockOpenBrainServer()
    server.start()
    while server.port == 0:
        pass
    try:
        yield server
    finally:
        server.stop()


def test_search_semantic_request_carries_embedding_selector_fields(mock_server):
    client = OpenBrainHttpClient(f"http://127.0.0.1:{mock_server.port}")
    client.search_semantic(
        SearchSemanticRequest(
            scope="work",
            query="query",
            top_k=3,
            embedding_provider="openai",
            embedding_model="text-embedding-3-small",
            embedding_kind="semantic",
        )
    )

    # validate the server got optional fields through the request body
    assert mock_server.httpd.last_semantic_body == {
        "scope": "work",
        "query": "query",
        "top_k": 3,
        "embedding_provider": "openai",
        "embedding_model": "text-embedding-3-small",
        "embedding_kind": "semantic",
    }


def test_error_maps_to_openbrain_error(mock_server):
    client = OpenBrainHttpClient(f"http://127.0.0.1:{mock_server.port}")
    with pytest.raises(OpenBrainError) as exc:
        client.read(ReadRequest(scope="work", refs=["r-missing"]))
    assert exc.value.code == "OB_NOT_FOUND"
    assert exc.value.details["missing_refs"] == ["r-missing"]

