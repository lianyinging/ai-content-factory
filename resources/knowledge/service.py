"""QA knowledge service: SQLite + Chroma + local BGE."""
from __future__ import annotations

import csv
import io
import json
from pathlib import Path
from typing import Any

import chroma_store
import db
import embed
from model_path import model_status


def capability() -> dict[str, Any]:
    status = model_status()
    chroma_ok = True
    chroma_msg = "Chroma 可用"
    try:
        import chromadb  # noqa: F401
    except Exception as exc:  # noqa: BLE001
        chroma_ok = False
        chroma_msg = f"Chroma 不可用: {exc}"
    available = bool(status.get("available")) and chroma_ok
    return {
        "ok": True,
        "available": available,
        "embedding": status,
        "chroma": {"available": chroma_ok, "message": chroma_msg},
        "message": (
            "QA 知识库就绪"
            if available
            else (status.get("message") if not status.get("available") else chroma_msg)
        ),
    }


def _tags_str(tags: list[str] | None) -> str:
    return ",".join(t for t in (tags or []) if t)


def _sync_embed(workspace: Path, base_id: str, item: dict[str, Any]) -> None:
    vector = embed.embed_query(item["question"])
    chroma_store.upsert_qa(
        workspace,
        base_id,
        qa_id=item["id"],
        question=item["question"],
        answer=item["answer"],
        embedding=vector,
        tags=_tags_str(item.get("tags")),
    )


def create_base(workspace: Path, *, name: str, description: str = "") -> dict[str, Any]:
    item = db.create_base(workspace, name=name, description=description)
    chroma_store.get_or_create_collection(workspace, item["id"])
    return item


def delete_base(workspace: Path, base_id: str) -> None:
    if not db.delete_base(workspace, base_id):
        raise ValueError("知识库不存在")
    chroma_store.delete_collection(workspace, base_id)


def create_qa(
    workspace: Path,
    base_id: str,
    *,
    question: str,
    answer: str,
    tags: list[str] | None = None,
) -> dict[str, Any]:
    item = db.insert_qa(workspace, base_id, question=question, answer=answer, tags=tags)
    _sync_embed(workspace, base_id, item)
    return item


def patch_qa(
    workspace: Path,
    base_id: str,
    qa_id: str,
    *,
    question: str | None = None,
    answer: str | None = None,
    tags: list[str] | None = None,
) -> dict[str, Any]:
    before = db.get_qa(workspace, base_id, qa_id)
    if not before:
        raise ValueError("问答不存在")
    item = db.update_qa(
        workspace,
        base_id,
        qa_id,
        question=question,
        answer=answer,
        tags=tags,
    )
    # Re-embed when question changes; otherwise refresh metadata with same embedding
    if question is not None and question.strip() != before["question"]:
        _sync_embed(workspace, base_id, item)
    else:
        vector = embed.embed_query(item["question"])
        chroma_store.upsert_qa(
            workspace,
            base_id,
            qa_id=item["id"],
            question=item["question"],
            answer=item["answer"],
            embedding=vector,
            tags=_tags_str(item.get("tags")),
        )
    return item


def remove_qa(workspace: Path, base_id: str, qa_id: str) -> None:
    if not db.delete_qa(workspace, base_id, qa_id):
        raise ValueError("问答不存在")
    chroma_store.delete_qa(workspace, base_id, qa_id)


def _parse_qa_blocks(text: str) -> list[dict[str, Any]] | None:
    """Parse FlowGame-style blocks:

    Q: question
    A: answer

    Q: ...
    A: ...
    """
    lines = text.replace("\r\n", "\n").replace("\r", "\n").split("\n")
    rows: list[dict[str, Any]] = []
    cur_q: list[str] = []
    cur_a: list[str] = []
    mode: str | None = None
    saw_marker = False

    def flush() -> None:
        nonlocal cur_q, cur_a, mode
        q = "\n".join(cur_q).strip()
        a = "\n".join(cur_a).strip()
        if q or a:
            rows.append({"question": q, "answer": a, "tags": []})
        cur_q, cur_a, mode = [], [], None

    for raw in lines:
        line = raw.rstrip()
        upper = line.lstrip()
        if upper.startswith("Q:") or upper.startswith("q:") or upper.startswith("问:"):
            saw_marker = True
            if cur_q or cur_a:
                flush()
            mode = "q"
            cur_q = [upper.split(":", 1)[1].lstrip()]
            continue
        if upper.startswith("A:") or upper.startswith("a:") or upper.startswith("答:"):
            saw_marker = True
            mode = "a"
            cur_a = [upper.split(":", 1)[1].lstrip()]
            continue
        if mode == "q":
            cur_q.append(line)
        elif mode == "a":
            cur_a.append(line)
    if cur_q or cur_a:
        flush()
    if not saw_marker:
        return None
    return rows


def import_qa(
    workspace: Path,
    base_id: str,
    *,
    raw_text: str = "",
    items: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    if not db.get_base(workspace, base_id):
        raise ValueError("知识库不存在")
    rows: list[dict[str, Any]] = []
    if items:
        rows = items
    else:
        text = (raw_text or "").strip()
        if not text:
            raise ValueError("导入内容为空")
        if text.startswith("["):
            parsed = json.loads(text)
            if not isinstance(parsed, list):
                raise ValueError("JSON 须为数组")
            rows = [x for x in parsed if isinstance(x, dict)]
        else:
            blocks = _parse_qa_blocks(text)
            if blocks is not None:
                rows = blocks
            else:
                reader = csv.DictReader(io.StringIO(text))
                rows = list(reader)

    created = 0
    errors: list[str] = []
    for idx, row in enumerate(rows, start=1):
        q = str(row.get("question") or row.get("q") or row.get("问") or "").strip()
        a = str(row.get("answer") or row.get("a") or row.get("答") or "").strip()
        tags_raw = row.get("tags") or row.get("tag") or ""
        if isinstance(tags_raw, list):
            tags = [str(t).strip() for t in tags_raw if str(t).strip()]
        else:
            tags = [t.strip() for t in str(tags_raw).split(",") if t.strip()]
        if not q or not a:
            errors.append(f"第 {idx} 行缺少 question/answer")
            continue
        try:
            create_qa(workspace, base_id, question=q, answer=a, tags=tags)
            created += 1
        except Exception as exc:  # noqa: BLE001
            errors.append(f"第 {idx} 行: {exc}")
    return {"ok": True, "created": created, "errors": errors}


def search(
    workspace: Path,
    *,
    base_id: str,
    query: str,
    top_k: int = 5,
) -> dict[str, Any]:
    if not db.get_base(workspace, base_id):
        raise ValueError("知识库不存在")
    q = (query or "").strip()
    if not q:
        raise ValueError("检索内容不能为空")
    vector = embed.embed_query(q)
    hits = chroma_store.query(workspace, base_id, embedding=vector, top_k=top_k)
    return {"ok": True, "query": q, "baseId": base_id, "items": hits}
