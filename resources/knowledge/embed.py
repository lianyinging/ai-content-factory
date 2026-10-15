"""Local BGE embedding via sentence-transformers (offline)."""
from __future__ import annotations

import json
import logging
import os
import threading
from typing import List

from model_path import resolve_local_model_path

logger = logging.getLogger("staff.knowledge.embed")

_lock = threading.RLock()
_model = None
_load_error: str | None = None


def _embedding_dim(module: object) -> int:
    for name in (
        "get_embedding_dimension",
        "get_word_embedding_dimension",
        "get_sentence_embedding_dimension",
    ):
        fn = getattr(module, name, None)
        if callable(fn):
            dim = fn()
            if dim is not None:
                return int(dim)
    raise AttributeError(f"{type(module).__name__} 无可用的 embedding dimension 方法")


def _load_sentence_transformer_local(model_path: str):
    from sentence_transformers import SentenceTransformer

    prev = {
        "HF_HUB_OFFLINE": os.environ.get("HF_HUB_OFFLINE"),
        "TRANSFORMERS_OFFLINE": os.environ.get("TRANSFORMERS_OFFLINE"),
        "HF_DATASETS_OFFLINE": os.environ.get("HF_DATASETS_OFFLINE"),
    }
    os.environ["HF_HUB_OFFLINE"] = "1"
    os.environ["TRANSFORMERS_OFFLINE"] = "1"
    os.environ["HF_DATASETS_OFFLINE"] = "1"
    try:
        try:
            from sentence_transformers.models import Pooling, Transformer

            config_path = os.path.join(model_path, "config_sentence_transformers.json")
            max_seq_length = 512
            if os.path.exists(config_path):
                with open(config_path, encoding="utf-8") as f:
                    st_config = json.load(f)
                max_seq_length = int(st_config.get("max_seq_length", 512))

            word_embedding_model = Transformer(model_path, max_seq_length=max_seq_length)
            pooling_model = Pooling(_embedding_dim(word_embedding_model))
            return SentenceTransformer(modules=[word_embedding_model, pooling_model])
        except Exception as exc:  # noqa: BLE001
            logger.warning("手动组装 SentenceTransformer 失败，尝试 local_files_only: %s", exc)
            return SentenceTransformer(model_path, local_files_only=True)
    finally:
        for key, value in prev.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value


def _ensure_model():
    global _model, _load_error
    with _lock:
        if _model is not None:
            return _model
        if _load_error:
            raise RuntimeError(_load_error)
        path = resolve_local_model_path()
        if path is None:
            _load_error = (
                "未找到本地 Embedding 模型，请复制 bge-small-zh-v1.5 到 resources/model/BAAI/"
            )
            raise RuntimeError(_load_error)
        try:
            logger.info("加载本地 Embedding: %s", path)
            _model = _load_sentence_transformer_local(str(path))
            _load_error = None
            return _model
        except Exception as exc:  # noqa: BLE001
            _load_error = f"加载 Embedding 失败: {exc}"
            raise RuntimeError(_load_error) from exc


def embed_documents(texts: List[str]) -> List[List[float]]:
    if not texts:
        return []
    model = _ensure_model()
    vectors = model.encode(texts, normalize_embeddings=True, show_progress_bar=False)
    return vectors.tolist()


def embed_query(text: str) -> List[float]:
    model = _ensure_model()
    vector = model.encode([text], normalize_embeddings=True, show_progress_bar=False)
    return vector[0].tolist()


def reset_for_tests() -> None:
    global _model, _load_error
    with _lock:
        _model = None
        _load_error = None
