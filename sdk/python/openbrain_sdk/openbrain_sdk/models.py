from __future__ import annotations

from dataclasses import asdict, dataclass, field
from typing import Any, Dict, List, Optional, Type, TypeVar, Union


Json = Dict[str, Any]
T = TypeVar("T", bound="FromDictMixin")


def _compact(value: Any) -> Any:
    if isinstance(value, dict):
        out: Dict[str, Any] = {}
        for key, val in value.items():
            compacted = _compact(val)
            if compacted is not None:
                out[key] = compacted
        return out
    if isinstance(value, list):
        return [_compact(v) for v in value]
    return value


class FromDictMixin:
    @classmethod
    def from_dict(cls: Type[T], value: Dict[str, Any]) -> T:
        return cls(**value)  # type: ignore[arg-type]


@dataclass
class ErrorEnvelope(FromDictMixin):
    code: str
    message: str
    details: Optional[Any] = None


@dataclass
class MemoryObject(FromDictMixin):
    type: Optional[str] = None
    id: Optional[str] = None
    scope: Optional[str] = None
    status: Optional[str] = None
    spec_version: Optional[str] = None
    tags: Optional[List[str]] = None
    data: Optional[Dict[str, Any]] = None
    provenance: Optional[Dict[str, Any]] = None

    def to_dict(self) -> Dict[str, Any]:
        return _compact(asdict(self))


@dataclass
class PutObjectsRequest(FromDictMixin):
    objects: List[MemoryObject]
    actor: Optional[str] = None
    idempotency_key: Optional[str] = None

    def to_dict(self) -> Dict[str, Any]:
        return {
            "objects": [obj.to_dict() for obj in self.objects],
            "actor": self.actor,
            "idempotency_key": self.idempotency_key,
        }


@dataclass
class PutResult(FromDictMixin):
    ref: str
    type: str
    status: str
    version: int


@dataclass
class PutObjectsResponse(FromDictMixin):
    results: List[PutResult]


@dataclass
class ReadRequest(FromDictMixin):
    scope: str
    refs: List[str]


@dataclass
class SearchStructuredRequest(FromDictMixin):
    scope: str
    where_expr: Optional[str] = None
    limit: Optional[int] = None
    offset: Optional[int] = None
    order_by: Optional[Json] = None

    def to_dict(self) -> Dict[str, Any]:
        return _compact(asdict(self))


@dataclass
class SearchStructuredResponse(FromDictMixin):
    results: List[Json]


@dataclass
class SearchSemanticRequest(FromDictMixin):
    scope: str
    query: str
    top_k: Optional[int] = None
    model: Optional[str] = None
    embedding_provider: Optional[str] = None
    embedding_model: Optional[str] = None
    embedding_kind: Optional[str] = None
    filters: Optional[str] = None
    types: Optional[List[str]] = None
    status: Optional[List[str]] = None

    def to_dict(self) -> Dict[str, Any]:
        return _compact(asdict(self))


@dataclass
class SearchSemanticResponse(FromDictMixin):
    matches: List[Json]


@dataclass
class EmbedText(FromDictMixin):
    text: str


@dataclass
class EmbedRef(FromDictMixin):
    ref: str


@dataclass
class EmbedGenerateRequest(FromDictMixin):
    scope: str
    target: Union[EmbedText, EmbedRef]
    model: str
    dims: Optional[int] = None

    def to_dict(self) -> Dict[str, Any]:
        target = asdict(self.target) if hasattr(self.target, "__dict__") else self.target
        payload: Dict[str, Any] = {
            "scope": self.scope,
            "target": target,
            "model": self.model,
            "dims": self.dims,
        }
        return _compact(payload)


@dataclass
class EmbedGenerateResponse(FromDictMixin):
    embedding_id: str
    object_id: Optional[str]
    model: str
    dims: int
    checksum: str
    reused: bool


@dataclass
class RerankRequest(FromDictMixin):
    scope: str
    query: str
    candidates: Optional[Any] = None
    top_k: Optional[int] = None

    def to_dict(self) -> Dict[str, Any]:
        return _compact(asdict(self))


@dataclass
class RerankResponse(FromDictMixin):
    ranked_refs: List[str]
    rationale_short: Optional[List[Json]] = None


@dataclass
class MemoryPackRequest(FromDictMixin):
    scope: str
    task_hint: str
    query: Optional[str] = None
    policy: Optional[Json] = None

    def to_dict(self) -> Dict[str, Any]:
        return _compact(asdict(self))


@dataclass
class MemoryPackResponse(FromDictMixin):
    pack: Json


@dataclass
class PingResponse(FromDictMixin):
    version: str
    server_time: str

