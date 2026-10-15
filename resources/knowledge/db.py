"""Local QA knowledge — SQLite metadata store."""
from __future__ import annotations

import json
import sqlite3
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


def _now() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat()


def staff_root(workspace: Path) -> Path:
    root = workspace / ".staff"
    root.mkdir(parents=True, exist_ok=True)
    return root


def db_path(workspace: Path) -> Path:
    return staff_root(workspace) / "knowledge.sqlite3"


def connect(workspace: Path) -> sqlite3.Connection:
    path = db_path(workspace)
    path.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(str(path))
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA foreign_keys = ON")
    _ensure_schema(conn)
    return conn


def _ensure_schema(conn: sqlite3.Connection) -> None:
    conn.executescript(
        """
        CREATE TABLE IF NOT EXISTS bases (
          base_id TEXT PRIMARY KEY,
          name TEXT NOT NULL,
          description TEXT NOT NULL DEFAULT '',
          kind TEXT NOT NULL DEFAULT 'qa',
          embedding_model TEXT NOT NULL DEFAULT 'bge-small-zh-v1.5',
          embedding_dim INTEGER NOT NULL DEFAULT 512,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS qa_items (
          qa_id TEXT PRIMARY KEY,
          base_id TEXT NOT NULL,
          question TEXT NOT NULL,
          answer TEXT NOT NULL,
          tags_json TEXT NOT NULL DEFAULT '[]',
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          FOREIGN KEY (base_id) REFERENCES bases(base_id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_qa_base ON qa_items(base_id);
        """
    )
    conn.commit()


def _base_row(row: sqlite3.Row) -> dict[str, Any]:
    return {
        "id": row["base_id"],
        "name": row["name"],
        "description": row["description"] or "",
        "kind": row["kind"] or "qa",
        "embeddingModel": row["embedding_model"],
        "embeddingDim": row["embedding_dim"],
        "createdAt": row["created_at"],
        "updatedAt": row["updated_at"],
    }


def _qa_row(row: sqlite3.Row) -> dict[str, Any]:
    try:
        tags = json.loads(row["tags_json"] or "[]")
    except json.JSONDecodeError:
        tags = []
    if not isinstance(tags, list):
        tags = []
    return {
        "id": row["qa_id"],
        "baseId": row["base_id"],
        "question": row["question"],
        "answer": row["answer"],
        "tags": [str(t) for t in tags],
        "createdAt": row["created_at"],
        "updatedAt": row["updated_at"],
    }


def list_bases(workspace: Path) -> list[dict[str, Any]]:
    with connect(workspace) as conn:
        rows = conn.execute(
            "SELECT b.*, (SELECT COUNT(*) FROM qa_items q WHERE q.base_id = b.base_id) AS qa_count "
            "FROM bases b ORDER BY b.updated_at DESC"
        ).fetchall()
        out = []
        for row in rows:
            item = _base_row(row)
            item["qaCount"] = int(row["qa_count"] or 0)
            out.append(item)
        return out


def get_base(workspace: Path, base_id: str) -> dict[str, Any] | None:
    with connect(workspace) as conn:
        row = conn.execute("SELECT * FROM bases WHERE base_id = ?", (base_id,)).fetchone()
        return _base_row(row) if row else None


def create_base(workspace: Path, *, name: str, description: str = "") -> dict[str, Any]:
    name_text = (name or "").strip()
    if not name_text:
        raise ValueError("知识库名称不能为空")
    now = _now()
    base_id = uuid.uuid4().hex
    with connect(workspace) as conn:
        conn.execute(
            "INSERT INTO bases (base_id, name, description, kind, embedding_model, embedding_dim, created_at, updated_at) "
            "VALUES (?, ?, ?, 'qa', 'bge-small-zh-v1.5', 512, ?, ?)",
            (base_id, name_text, (description or "").strip(), now, now),
        )
        conn.commit()
    item = get_base(workspace, base_id)
    assert item is not None
    item["qaCount"] = 0
    return item


def delete_base(workspace: Path, base_id: str) -> bool:
    with connect(workspace) as conn:
        cur = conn.execute("DELETE FROM bases WHERE base_id = ?", (base_id,))
        conn.commit()
        return cur.rowcount > 0


def list_qa(
    workspace: Path,
    base_id: str,
    *,
    page: int = 1,
    page_size: int = 20,
    q: str = "",
) -> dict[str, Any]:
    page = max(1, int(page or 1))
    page_size = max(1, min(int(page_size or 20), 100))
    offset = (page - 1) * page_size
    keyword = (q or "").strip()
    with connect(workspace) as conn:
        if keyword:
            like = f"%{keyword}%"
            total = conn.execute(
                "SELECT COUNT(*) AS c FROM qa_items WHERE base_id = ? AND (question LIKE ? OR answer LIKE ?)",
                (base_id, like, like),
            ).fetchone()["c"]
            rows = conn.execute(
                "SELECT * FROM qa_items WHERE base_id = ? AND (question LIKE ? OR answer LIKE ?) "
                "ORDER BY updated_at DESC LIMIT ? OFFSET ?",
                (base_id, like, like, page_size, offset),
            ).fetchall()
        else:
            total = conn.execute(
                "SELECT COUNT(*) AS c FROM qa_items WHERE base_id = ?", (base_id,)
            ).fetchone()["c"]
            rows = conn.execute(
                "SELECT * FROM qa_items WHERE base_id = ? ORDER BY updated_at DESC LIMIT ? OFFSET ?",
                (base_id, page_size, offset),
            ).fetchall()
        return {
            "items": [_qa_row(r) for r in rows],
            "total": int(total or 0),
            "page": page,
            "pageSize": page_size,
        }


def get_qa(workspace: Path, base_id: str, qa_id: str) -> dict[str, Any] | None:
    with connect(workspace) as conn:
        row = conn.execute(
            "SELECT * FROM qa_items WHERE base_id = ? AND qa_id = ?",
            (base_id, qa_id),
        ).fetchone()
        return _qa_row(row) if row else None


def insert_qa(
    workspace: Path,
    base_id: str,
    *,
    question: str,
    answer: str,
    tags: list[str] | None = None,
    qa_id: str | None = None,
) -> dict[str, Any]:
    q = (question or "").strip()
    a = (answer or "").strip()
    if not q:
        raise ValueError("问题不能为空")
    if not a:
        raise ValueError("答案不能为空")
    if not get_base(workspace, base_id):
        raise ValueError("知识库不存在")
    now = _now()
    item_id = (qa_id or "").strip() or uuid.uuid4().hex
    tag_list = [str(t).strip() for t in (tags or []) if str(t).strip()]
    with connect(workspace) as conn:
        conn.execute(
            "INSERT INTO qa_items (qa_id, base_id, question, answer, tags_json, created_at, updated_at) "
            "VALUES (?, ?, ?, ?, ?, ?, ?)",
            (item_id, base_id, q, a, json.dumps(tag_list, ensure_ascii=False), now, now),
        )
        conn.execute("UPDATE bases SET updated_at = ? WHERE base_id = ?", (now, base_id))
        conn.commit()
    item = get_qa(workspace, base_id, item_id)
    assert item is not None
    return item


def update_qa(
    workspace: Path,
    base_id: str,
    qa_id: str,
    *,
    question: str | None = None,
    answer: str | None = None,
    tags: list[str] | None = None,
) -> dict[str, Any]:
    existing = get_qa(workspace, base_id, qa_id)
    if not existing:
        raise ValueError("问答不存在")
    q = existing["question"] if question is None else (question or "").strip()
    a = existing["answer"] if answer is None else (answer or "").strip()
    if not q:
        raise ValueError("问题不能为空")
    if not a:
        raise ValueError("答案不能为空")
    tag_list = (
        existing["tags"]
        if tags is None
        else [str(t).strip() for t in tags if str(t).strip()]
    )
    now = _now()
    with connect(workspace) as conn:
        conn.execute(
            "UPDATE qa_items SET question = ?, answer = ?, tags_json = ?, updated_at = ? "
            "WHERE base_id = ? AND qa_id = ?",
            (q, a, json.dumps(tag_list, ensure_ascii=False), now, base_id, qa_id),
        )
        conn.execute("UPDATE bases SET updated_at = ? WHERE base_id = ?", (now, base_id))
        conn.commit()
    item = get_qa(workspace, base_id, qa_id)
    assert item is not None
    return item


def delete_qa(workspace: Path, base_id: str, qa_id: str) -> bool:
    with connect(workspace) as conn:
        cur = conn.execute(
            "DELETE FROM qa_items WHERE base_id = ? AND qa_id = ?",
            (base_id, qa_id),
        )
        if cur.rowcount:
            conn.execute(
                "UPDATE bases SET updated_at = ? WHERE base_id = ?",
                (_now(), base_id),
            )
        conn.commit()
        return cur.rowcount > 0


def all_base_names(workspace: Path) -> list[dict[str, str]]:
    with connect(workspace) as conn:
        rows = conn.execute("SELECT base_id, name FROM bases ORDER BY name ASC").fetchall()
        return [{"id": r["base_id"], "name": r["name"]} for r in rows]
