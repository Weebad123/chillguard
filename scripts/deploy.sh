#!/usr/bin/env bash
# Deploy the built ChillGuard cells to a running Myrmic runtime.
#
# Usage: scripts/deploy.sh [cell...]  (run `make build-cells` first)
#   With no arguments every cell is deployed; otherwise only the named ones.
#
# Environment:
#   SRN_PREFIX      prefix for every cell name, e.g. "e2e-123-"   (default: none)
#   DEPLOY_TAG      placement tag the target runtime must carry  (default: none)
#   MYRMIC_TIMEOUT  per-command timeout                          (default: 30s)
#   CONFIG_DIR      directory of the init payloads                (default: config)
#   MYRMIC_CONNECT  runtime endpoint for `--connect`, e.g. tcp/127.0.0.1:20123
#                   (default: none, multicast discovery)
#
# A cell that takes an init payload gets it from $CONFIG_DIR/<cell>.dev.json,
# if that file exists.
#
# A running cell's code cannot be swapped in place, so a cell that is already
# deployed under the same name is deleted first.
set -euo pipefail

cd "$(dirname "$0")/.."
# shellcheck source=scripts/cells.sh
source scripts/cells.sh

prefix="${SRN_PREFIX:-}"
config_dir="${CONFIG_DIR:-config}"
connect_args=()
if [[ -n "${MYRMIC_CONNECT:-}" ]]; then
    connect_args=(--connect "$MYRMIC_CONNECT")
fi
timeout="${MYRMIC_TIMEOUT:-30s}"
tag_args=()
if [[ -n "${DEPLOY_TAG:-}" ]]; then
    tag_args=(--tag "$DEPLOY_TAG")
fi

if [[ $# -gt 0 ]]; then
    CELLS=("$@")
fi

for cell in "${CELLS[@]}"; do
    wasm="$(wasm_path "$cell")"
    if [[ ! -f "$wasm" ]]; then
        echo "error: $wasm not found; run 'make build-cells' first" >&2
        exit 1
    fi
    # A binary older than the sources it was built from is not what the
    # tree says it is. Seen once: a build reported `Finished` and left an
    # older artifact in place, and the deploy then failed in init. Test-only
    # sources (`tests/` directories, `tests.rs` modules) and the operator CLI
    # are not part of a cell and are left out, or cargo's correct "nothing
    # to do" would be refused here.
    stale="$(find crates -name '*.rs' -not -path 'crates/chillguard-cli/*' -not -path '*/tests/*' -not -name tests.rs -newer "$wasm" -print -quit)"
    if [[ -n "$stale" ]]; then
        echo "error: $wasm is older than $stale; run 'make build-cells'" >&2
        exit 1
    fi
    srn="${prefix}${cell}"
    init_args=()
    if [[ -f "${config_dir}/${cell}.dev.json" ]]; then
        init_args=(--init-file "${config_dir}/${cell}.dev.json")
    fi
    # Fails when nothing is deployed under that name, which is the normal
    # first-deploy case. A real failure resurfaces as a deploy error below.
    myrmic --timeout "$timeout" "${connect_args[@]}" delete "$srn" --cell >/dev/null 2>&1 || true
    myrmic --timeout "$timeout" "${connect_args[@]}" deploy "$wasm" --name "$srn" "${tag_args[@]}" "${init_args[@]}"
done
