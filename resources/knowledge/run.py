#!/usr/bin/env python3
"""Knowledge sidecar — stdin JSON command dispatcher for Tauri."""
from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))


def dispatch(payload: dict) -> dict:
    import db
    import service

    action = str(payload.get("action") or "").strip()
    ws_raw = (payload.get("workspace") or "").strip()
    workspace = Path(ws_raw).expanduser() if ws_raw else Path.home() / ".ai-content-factory" / "knowledge"
    workspace.mkdir(parents=True, exist_ok=True)

    if action == "capability":
        return service.capability()

    if action == "list_bases":
        return {"ok": True, "items": db.list_bases(workspace)}

    if action in ("create_base", "upsert_base"):
        item = service.create_base(
            workspace,
            name=str(payload.get("name") or ""),
            description=str(payload.get("description") or ""),
        )
        return {"ok": True, "item": item}

    if action == "delete_base":
        service.delete_base(workspace, str(payload.get("baseId") or ""))
        return {"ok": True}

    if action == "list_qa":
        data = db.list_qa(
            workspace,
            str(payload.get("baseId") or ""),
            page=int(payload.get("page") or 1),
            page_size=int(payload.get("pageSize") or 50),
            q=str(payload.get("q") or ""),
        )
        return {"ok": True, **data}

    if action in ("create_qa", "upsert_qa"):
        base_id = str(payload.get("baseId") or "")
        qa_id = str(payload.get("qaId") or payload.get("id") or "").strip()
        question = str(payload.get("question") or "")
        answer = str(payload.get("answer") or "")
        tags = payload.get("tags") if isinstance(payload.get("tags"), list) else None
        if qa_id and db.get_qa(workspace, base_id, qa_id):
            item = service.patch_qa(
                workspace,
                base_id,
                qa_id,
                question=question or None,
                answer=answer or None,
                tags=tags,
            )
        else:
            item = service.create_qa(
                workspace, base_id, question=question, answer=answer, tags=tags
            )
        return {"ok": True, "item": item}

    if action == "delete_qa":
        service.remove_qa(
            workspace,
            str(payload.get("baseId") or ""),
            str(payload.get("qaId") or payload.get("id") or ""),
        )
        return {"ok": True}

    if action == "import_qa":
        return service.import_qa(
            workspace,
            str(payload.get("baseId") or ""),
            raw_text=str(payload.get("rawText") or ""),
            items=payload.get("items") if isinstance(payload.get("items"), list) else None,
        )

    if action == "search":
        base_ids = payload.get("baseIds") or []
        if isinstance(base_ids, str):
            base_ids = [base_ids]
        base_id = str(payload.get("baseId") or "").strip()
        if base_id:
            base_ids = [base_id]
        query = str(payload.get("query") or payload.get("q") or "")
        top_k = int(payload.get("topK") or 5)
        items = []
        for bid in base_ids:
            bid = str(bid or "").strip()
            if not bid:
                continue
            result = service.search(workspace, base_id=bid, query=query, top_k=top_k)
            for hit in result.get("items") or []:
                row = dict(hit)
                row["baseId"] = bid
                items.append(row)
        items.sort(key=lambda x: float(x.get("score") or 0), reverse=True)
        return {"ok": True, "query": query, "items": items[: max(1, min(top_k, 20))]}

    raise ValueError(f"unknown action: {action}")


def main() -> int:
    raw = sys.stdin.read()
    try:
        payload = json.loads(raw or "{}")
    except json.JSONDecodeError as exc:
        print(json.dumps({"ok": False, "error": f"invalid json: {exc}"}, ensure_ascii=False))
        return 1
    if not isinstance(payload, dict):
        print(json.dumps({"ok": False, "error": "payload must be object"}, ensure_ascii=False))
        return 1
    try:
        result = dispatch(payload)
        print(json.dumps(result, ensure_ascii=False))
        return 0 if result.get("ok", True) is not False else 1
    except Exception as exc:  # noqa: BLE001
        print(json.dumps({"ok": False, "error": str(exc)}, ensure_ascii=False))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
