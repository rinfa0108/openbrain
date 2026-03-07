import time

from openbrain_sdk import OpenBrainHttpClient
from openbrain_sdk.models import (
    EmbedGenerateRequest,
    EmbedRef,
    MemoryObject,
    MemoryPackRequest,
    PutObjectsRequest,
    ReadRequest,
    SearchSemanticRequest,
)


def main() -> None:
    client = OpenBrainHttpClient()
    scope = "sdk-usage-proof"
    obj_id = f"decision-{int(time.time())}"

    client.ping()

    write = client.write(
        PutObjectsRequest(
            objects=[
                MemoryObject(
                    type="decision",
                    id=obj_id,
                    scope=scope,
                    status="canonical",
                    spec_version="0.1",
                    tags=["sdk", "usage-proof"],
                    data={"title": "Adopt OpenBrain SDKs", "reason": "Developer ergonomics"},
                    provenance={"actor": "sdk-usage-proof", "method": "http-e2e"},
                )
            ]
        )
    )

    ref = write.results[0].ref if write.results else obj_id
    read = client.read(ReadRequest(scope=scope, refs=[ref]))
    print(f"read objects: {len(read.objects)}")

    embed = client.embed_generate(
        EmbedGenerateRequest(scope=scope, target=EmbedRef(ref=ref), model="default")
    )
    print(f"embedding reused: {embed.reused}")

    semantic = client.search_semantic(
        SearchSemanticRequest(
            scope=scope,
            query="developer ergonomics",
            top_k=5,
            embedding_provider="fake",
            embedding_model="default",
            embedding_kind="semantic",
        )
    )
    print(f"semantic matches: {len(semantic.matches)}")

    try:
        pack = client.memory_pack(
            MemoryPackRequest(scope=scope, task_hint="Summarize current decisions")
        )
        print(f"pack summary: {pack.pack.get('summary')}")
    except Exception as exc:
        print(f"memory.pack skipped: {exc}")


if __name__ == "__main__":
    main()
