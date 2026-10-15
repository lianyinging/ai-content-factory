#!/usr/bin/env bash
# Create an isolated venv for knowledge sidecar (NumPy 1.x + Chroma).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"
PYTHON="${PYTHON:-python3}"
"$PYTHON" -m venv .venv
# shellcheck disable=SC1091
source .venv/bin/activate
python -m pip install -U pip
python -m pip install -r requirements.txt
python - <<'PY'
import numpy
import chromadb
print("ok numpy", numpy.__version__, "chromadb", getattr(chromadb, "__version__", "?"))
PY
echo "Knowledge venv ready: $ROOT/.venv"
