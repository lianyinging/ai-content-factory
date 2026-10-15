"""Resolve local BGE embedding model path (HF cache / snapshot compatible)."""
from __future__ import annotations

import os
from pathlib import Path

# resources/knowledge -> resources
_RESOURCES_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_LOCAL_MODEL_DIR = _RESOURCES_ROOT / "model" / "BAAI" / "bge-small-zh-v1.5"
REMOTE_FALLBACK_MODEL = "BAAI/bge-small-zh-v1.5"
EMBEDDING_DIM = 512


def _find_local_model_dir(base_dir: Path) -> Path | None:
    """Recursively find a dir containing config.json / modules.json (HF snapshots)."""
    if not base_dir.is_dir():
        return None
    if (base_dir / "config.json").is_file() or (base_dir / "modules.json").is_file():
        return base_dir
    for root, _dirs, files in os.walk(base_dir):
        if "config.json" in files or "sentence_bert_config.json" in files or "modules.json" in files:
            return Path(root)
    return None


def resolve_local_model_path() -> Path | None:
    candidates: list[Path] = []
    env_path = (os.getenv("EMBEDDING_MODEL_PATH") or "").strip()
    if env_path:
        candidates.append(Path(env_path).expanduser())
    candidates.append(DEFAULT_LOCAL_MODEL_DIR)

    for base in candidates:
        found = _find_local_model_dir(base.resolve() if base.exists() else base)
        if found:
            return found
    return None


def model_status() -> dict:
    path = resolve_local_model_path()
    if path is None:
        return {
            "available": False,
            "path": str(DEFAULT_LOCAL_MODEL_DIR),
            "message": (
                f"未找到本地 Embedding 模型。请将 bge-small-zh-v1.5 复制到 "
                f"{DEFAULT_LOCAL_MODEL_DIR}（可从数字员工 super_staff/model/BAAI 复制）。"
            ),
            "dim": EMBEDDING_DIM,
        }
    return {
        "available": True,
        "path": str(path),
        "message": "本地 BGE embedding 就绪",
        "dim": EMBEDDING_DIM,
        "model": "bge-small-zh-v1.5",
    }
