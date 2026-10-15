"""Chroma PersistentClient helpers for QA knowledge bases."""
from __future__ import annotations

from pathlib import Path
from typing import Any

from model_path import EMBEDDING_DIM


def chroma_root(workspace: Path) -> Path:
    root = workspace / ".staff" / "chroma"
    root.mkdir(parents=True, exist_ok=True)
    return root


def collection_name(base_id: str) -> str:
    safe = "".join(ch if ch.isalnum() or ch in "-_" else "_" for ch in base_id)
    return f"kb_{safe}"[:200]


def _client(workspace: Path):
    import chromadb
    from chromadb.config import Settings

    return chromadb.PersistentClient(
        path=str(chroma_root(workspace)),
        settings=Settings(anonymized_telemetry=False, allow_reset=True),
    )


def get_or_create_collection(workspace: Path, base_id: str):
    client = _client(workspace)
    return client.get_or_create_collection(
        name=collection_name(base_id),
        metadata={"hnsw:space": "cosine", "dim": str(EMBEDDING_DIM)},
    )


def delete_collection(workspace: Path, base_id: str) -> None:
    client = _client(workspace)
    name = collection_name(base_id)
    try:
        client.delete_collection(name)
    except Exception:  # noqa: BLE001
        pass


def upsert_qa(
    workspace: Path,
    base_id: str,
    *,
    qa_id: str,
    question: str,
    answer: str,
    embedding: list[float],
    tags: str = "",
) -> None:
    col = get_or_create_collection(workspace, base_id)
    col.upsert(
        ids=[qa_id],
        embeddings=[embedding],
        documents=[question],
        metadatas=[
            {
                "qa_id": qa_id,
                "question": question[:2000],
                "answer": answer[:8000],
                "tags": (tags or "")[:500],
            }
        ],
    )


def delete_qa(workspace: Path, base_id: str, qa_id: str) -> None:
    col = get_or_create_collection(workspace, base_id)
    try:
        col.delete(ids=[qa_id])
    except Exception:  # noqa: BLE001
        pass


def query(
    workspace: Path,
    base_id: str,
    *,
    embedding: list[float],
    top_k: int = 5,
) -> list[dict[str, Any]]:
    col = get_or_create_collection(workspace, base_id)
    count = col.count()
    if count <= 0:
        return []
    limit = max(1, min(int(top_k or 5), 20, count))
    result = col.query(
        query_embeddings=[embedding],
        n_results=limit,
        include=["documents", "metadatas", "distances"],
    )
    ids = (result.get("ids") or [[]])[0]
    docs = (result.get("documents") or [[]])[0]
    metas = (result.get("metadatas") or [[]])[0]
    dists = (result.get("distances") or [[]])[0]
    out: list[dict[str, Any]] = []
    for i, qa_id in enumerate(ids):
        meta = metas[i] if i < len(metas) and isinstance(metas[i], dict) else {}
        dist = float(dists[i]) if i < len(dists) else 1.0
        score = max(0.0, 1.0 - dist)
        out.append(
            {
                "id": meta.get("qa_id") or qa_id,
                "question": meta.get("question") or (docs[i] if i < len(docs) else ""),
                "answer": meta.get("answer") or "",
                "tags": meta.get("tags") or "",
                "score": round(score, 4),
                "distance": round(dist, 4),
            }
        )
    return out
