#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
# Explicit manual staging only. Settle protocol-2 beta with its retained matching
# helper and stop its daemon first. This neither modifies beta assets/formula nor
# offers a protocol-3 bridge to a protocol-2 direct updater.
exec bash "$ROOT_DIR/scripts/install.sh" --pinned-0.3.6 "$@"
