from __future__ import annotations

import json
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Any, Optional, Type, TypeVar

from .error import OpenBrainError
from .models import (
    EmbedGenerateRequest,
    EmbedGenerateResponse,
    ErrorEnvelope,
    MemoryPackRequest,
    MemoryPackResponse,
    PingResponse,
    PutObjectsRequest,
    PutObjectsResponse,
    SearchSemanticRequest,
    SearchSemanticResponse,
    SearchStructuredRequest,
    SearchStructuredResponse,
    ReadRequest,
)

Json = dict[str, Any] | list[Any] | str | int | float | bool | None
T = TypeVar("T")


@dataclass
class _GetObjectsObject:
    type: str
    id: str
    scope: str
    status: str
    spec_version: str
    tags: list[str]
    data: Json
    provenance: Json
    version: int
    created_at: str
    updated_at: str


@dataclass
class GetObjectsResponse:
    objects: list[_GetObjectsObject]

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "GetObjectsResponse":
        objs = [_GetObjectsObject(**item) for item in payload["objects"]]
        return cls(objects=objs)


class OpenBrainHttpClient:
    def __init__(self, base_url: str = "http://127.0.0.1:7981", timeout: float = 15.0):
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout

    def ping(self) -> PingResponse:
        payload = self._post("/v1/ping", {})
        return PingResponse.from_dict(payload)  # type: ignore[attr-defined]

    def write(self, req: PutObjectsRequest) -> PutObjectsResponse:
        payload = self._post("/v1/write", req.to_dict())
        return PutObjectsResponse.from_dict(payload)

    def read(self, req: ReadRequest) -> GetObjectsResponse:
        payload = self._post("/v1/read", {"scope": req.scope, "refs": req.refs})
        return GetObjectsResponse.from_dict(payload)

    def search_structured(self, req: SearchStructuredRequest) -> SearchStructuredResponse:
        payload = self._post("/v1/search/structured", req.to_dict())
        return SearchStructuredResponse.from_dict(payload)

    def embed_generate(self, req: EmbedGenerateRequest) -> EmbedGenerateResponse:
        payload = self._post("/v1/embed/generate", req.to_dict())
        return EmbedGenerateResponse.from_dict(payload)

    def search_semantic(self, req: SearchSemanticRequest) -> SearchSemanticResponse:
        payload = self._post("/v1/search/semantic", req.to_dict())
        return SearchSemanticResponse.from_dict(payload)

    def rerank(self, req) -> Any:
        # Keep request/response loosely typed to avoid pulling a model dependency for this endpoint.
        return self._post("/v1/rerank", {"scope": req.scope, "query": req.query, "candidates": req.candidates, "top_k": req.top_k})

    def memory_pack(self, req: MemoryPackRequest) -> MemoryPackResponse:
        payload = self._post("/v1/memory/pack", req.to_dict())
        return MemoryPackResponse.from_dict(payload)

    def _post(self, path: str, body: dict[str, Any]) -> dict[str, Any]:
        url = f"{self.base_url}{path}"
        data = json.dumps(body).encode("utf-8")
        request = urllib.request.Request(url, data=data, method="POST", headers={"content-type": "application/json"})
        try:
            with urllib.request.urlopen(request, timeout=self.timeout) as response:
                raw = response.read().decode("utf-8")
        except urllib.error.HTTPError as err:
            raw = err.read().decode("utf-8")
            payload = self._parse_json(raw)
            if isinstance(payload, dict) and not payload.get("ok", True) and isinstance(payload.get("error"), dict):
                self._raise_error(payload["error"], err.code)
            raise OpenBrainError("HTTP_ERROR", str(err), status=getattr(err, "code", None), details={"body": raw})
        except urllib.error.URLError as err:
            raise OpenBrainError("NETWORK_ERROR", str(err))

        payload = self._parse_json(raw)
        if not isinstance(payload, dict) or payload.get("ok") is None:
            raise OpenBrainError("OB_INTERNAL", "invalid response payload", status=None, details={"raw": raw})

        if payload.get("ok") is False:
            error_payload = payload.get("error")
            if isinstance(error_payload, dict):
                self._raise_error(error_payload, None)
            raise OpenBrainError("OB_INTERNAL", "unexpected error payload", details=payload)

        return {key: value for key, value in payload.items() if key != "ok"}

    def _parse_json(self, raw: str) -> dict[str, Any] | list[Any] | str:
        try:
            return json.loads(raw)
        except json.JSONDecodeError as err:
            raise OpenBrainError("OB_INTERNAL", "invalid JSON response", details={"error": str(err), "raw": raw})

    def _raise_error(self, envelope: dict[str, Any], status: Optional[int]) -> None:
        code = str(envelope.get("code", "OB_INTERNAL"))
        message = str(envelope.get("message", "error"))
        details = envelope.get("details")
        raise OpenBrainError(code, message, status=status, details=details)

