#!/usr/bin/env bash
# Deploy the built ChillGuard cells to a running Myrmic runtime.
#
# Usage: scripts/deploy.sh            (run `make build-cells` first)
#
# Environment:
#   SRN_PREFIX      prefix for every cell name, e.g. "e2e-123-"   (default: none)
#   DEPLOY_TAG      placement tag the target runtime must carry  (default: none)
#   MYRMIC_TIMEOUT  per-command timeout                          (default: 30s)
#
# A running cell's code cannot be swapped in place, so a cell that is already
# deployed under the same name is deleted first.
set -euo pipefail

cd "$(dirname "$0")/.."
# shellcheck source=scripts/cells.sh
source scripts/cells.sh

prefix="${SRN_PREFIX:-}"
timeout="${MYRMIC_TIMEOUT:-30s}"
tag_args=()
if [[ -n "${DEPLOY_TAG:-}" ]]; then
    tag_args=(--tag "$DEPLOY_TAG")
fi

for cell in "${CELLS[@]}"; do
    wasm="$(wasm_path "$cell")"
    if [[ ! -f "$wasm" ]]; then
        echo "error: $wasm not found; run 'make build-cells' first" >&2
        exit 1
    fi
    srn="${prefix}${cell}"
    # Fails when nothing is deployed under that name, which is the normal
    # first-deploy case. A real failure resurfaces as a deploy error below.
    myrmic --timeout "$timeout" delete "$srn" --cell >/dev/null 2>&1 || true
    myrmic --timeout "$timeout" deploy "$wasm" --name "$srn" "${tag_args[@]}"
done
