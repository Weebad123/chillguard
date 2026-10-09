#!/usr/bin/env bash
# End-to-end smoke test: start a throwaway Myrmic runtime, deploy every cell,
# and check that each one is registered and ran its init handler.
#
# Usage: scripts/e2e.sh               (run `make build-cells` first)
#
# The run is isolated from anything else on the machine or the network:
#   - the runtime keeps its files in a temporary XDG_DATA_HOME, with an
#     in-memory database;
#   - the runtime carries a tag unique to this run and every cell requires it,
#     so cells cannot land on another runtime;
#   - cell names carry a prefix unique to this run.
set -euo pipefail

cd "$(dirname "$0")/.."
# shellcheck source=scripts/cells.sh
source scripts/cells.sh

run_id="e2e-$$"
runtime="chillguard-${run_id}"
timeout="${MYRMIC_TIMEOUT:-30s}"
XDG_DATA_HOME="$(mktemp -d)"
export XDG_DATA_HOME
export SRN_PREFIX="${run_id}-"
export DEPLOY_TAG="$runtime"

cleanup() {
    status=$?
    if [[ $status -ne 0 ]]; then
        echo "--- e2e failed; last runtime log lines:" >&2
        myrmic runtimes "$runtime" logs 2>/dev/null | tail -n 30 >&2 || true
    fi
    for cell in "${CELLS[@]}"; do
        myrmic --timeout "$timeout" delete "${SRN_PREFIX}${cell}" --cell >/dev/null 2>&1 || true
    done
    myrmic runtimes delete "$runtime" >/dev/null 2>&1 || true
    rm -rf "$XDG_DATA_HOME"
    exit "$status"
}
trap cleanup EXIT

# Retry a check once per second until it passes or 30 attempts are used up.
wait_for() {
    local what="$1"
    shift
    for _ in $(seq 1 30); do
        if "$@"; then
            return 0
        fi
        sleep 1
    done
    echo "error: timed out waiting for ${what}" >&2
    return 1
}

runtime_ready() {
    myrmic --timeout 2s network --once 2>/dev/null | grep -q -- "$runtime"
}

cell_started() {
    myrmic runtimes "$runtime" logs 2>/dev/null | grep -q -- "$1 started"
}

echo "==> starting runtime ${runtime}"
myrmic runtimes start config/runtime.dev.yml \
    --name "$runtime" --tag "$runtime" --tmp --detached
wait_for "runtime ${runtime} to join the network" runtime_ready

echo "==> deploying cells"
scripts/deploy.sh

echo "==> checking cells"
listing="$(myrmic --timeout "$timeout" cells --once)"
for cell in "${CELLS[@]}"; do
    srn="${SRN_PREFIX}${cell}"
    if ! grep -q -- "$srn" <<<"$listing"; then
        echo "error: ${srn} is not registered" >&2
        echo "$listing" >&2
        exit 1
    fi
    wait_for "${cell} to log its start" cell_started "$cell"
    echo "ok: ${srn} is registered and started"
done

echo "==> e2e passed"
