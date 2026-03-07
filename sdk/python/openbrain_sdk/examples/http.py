from openbrain_sdk import OpenBrainHttpClient
from openbrain_sdk.models import MemoryObject, PutObjectsRequest, SearchSemanticRequest


def main() -> None:
    client = OpenBrainHttpClient()
    client.write(
        PutObjectsRequest(
            objects=[
                MemoryObject(
                    type="decision",
                    id="decision-001",
                    scope="project-alpha",
                    status="canonical",
                    spec_version="0.1",
                    tags=["example", "python"],
                    data={"decision": "Adopt SDKs"},
                    provenance={"actor": "example"},
                )
            ]
        )
    )
    client.search_semantic(
        SearchSemanticRequest(
            scope="project-alpha",
            query="adoption criteria",
            top_k=5,
            embedding_provider="openai",
            embedding_model="text-embedding-3-small",
            embedding_kind="semantic",
        )
    )


if __name__ == "__main__":
    main()

