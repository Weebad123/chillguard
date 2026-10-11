#!/usr/bin/env bash
# End-to-end test harness: a throwaway Myrmic runtime, all three cells, and
# the simulator's scenarios played at 60x (config/e2e/).
#
# Usage: scripts/e2e.sh [scenario...]       (run `make build-cells` first)
#   scenarios: normal excursion stale restart        (default: all)
#
# Every scenario deploys the cells under its own SRN prefix, so each starts
# from empty state, and deletes them when it is done. The runtime keeps a
# persistent database in a temporary XDG_DATA_HOME (needed by `restart`),
# carries a tag unique to this run that every cell requires, and is removed
# at the end whatever happened. On a failed assertion the relevant cell log
# lines and the runtime's warnings are printed.
#
# The cells are driven and queried through the operator CLI (`chillguard`,
# crates/chillguard-cli), so every run also tests it against a real runtime.
# What happened inside the cells is asserted on their log lines.
#
# Events travel the whole network, not one runtime, and state has been seen
# to go missing while other nodes were around. The runtime is therefore kept
# off the network: multicast scouting disabled, listening on loopback only
# (config/e2e/runtime.yml), and every CLI call reaches it with --connect.
# The cells also get a refrigerator_id unique to this run, written into
# copies of config/e2e/, so that even a shared network could not feed them
# another run's readings.
set -euo pipefail

cd "$(dirname "$0")/.."
# shellcheck source=scripts/cells.sh
source scripts/cells.sh

run_id="e2e-$$"
runtime="chillguard-${run_id}"
timeout="${MYRMIC_TIMEOUT:-30s}"
XDG_DATA_HOME="$(mktemp -d)"
export XDG_DATA_HOME
fridge="fridge-${run_id}"
port=$(( 20000 + $$ % 10000 ))
export MYRMIC_CONNECT="tcp/127.0.0.1:${port}"
# config/e2e/*.json name the refrigerator `fridge-e2e` and runtime.yml a port
# `E2E_PORT`; this run's copies name both after the run.
CONFIG_DIR="${XDG_DATA_HOME}/config"
mkdir -p "$CONFIG_DIR"
for file in config/e2e/*.json; do
    sed "s/\"fridge-e2e\"/\"${fridge}\"/" "$file" > "${CONFIG_DIR}/$(basename "$file")"
done
sed "s/E2E_PORT/${port}/" config/e2e/runtime.yml > "${CONFIG_DIR}/runtime.yml"
export CONFIG_DIR
# The e2e payloads are the dev profile (config/e2e/<cell>.dev.json): the
# harness drives the sensor with `simulate`, which only dev mode accepts.
export PROFILE=dev
export DEPLOY_TAG="$runtime"
export MYRMIC_TIMEOUT="$timeout"

# Every CLI call that talks to the runtime goes straight to it.
m() {
    myrmic --timeout "$timeout" --connect "$MYRMIC_CONNECT" "$@"
}
# From config/e2e: readings every 1 s at 60x; critical after 5 s; stale after 10 s.
critical_after_ms=5000
stale_after_ms=10000

scenario=""               # the scenario being run, for messages and SRNs
scenario_since=""         # when it began; log lines before it belong to another
runtime_started=false     # nothing to tear down until start_runtime ran
failures=0

# ---------------------------------------------------------------------------
# Output
# ---------------------------------------------------------------------------

say() { printf '%s %s\n' "$(date -u +%H:%M:%S)" "$*"; }
step() { say "==> [$scenario] $*"; }

# Cell log lines of the current scenario, with the host's fields trimmed off.
# Timestamps sort as text, so a comparison on the first column scopes them.
cell_log() {
    myrmic runtimes "$runtime" logs 2>/dev/null | grep wasm_log \
        | awk -v since="$scenario_since" '$1 >= since' \
        | sed -E 's/^([^ ]+) +([A-Z]+) .*logging: /\1 \2 /; s/ sri=.*//' || true
}

runtime_warnings() {
    myrmic runtimes "$runtime" logs 2>/dev/null | grep -E ' (WARN|ERROR) ' \
        | grep -vE 'tag overlay|zenoh|opentelemetry|wasm_log' \
        | sed -E 's/ uptime_[a-z_]+=[0-9]+//g; s/ zid=[^ ]+//; s/ node_id=[^ ]+//' || true
}

# Records a failed assertion with the evidence, and carries on with the
# scenario so that one failure does not hide the next.
fail() {
    failures=$((failures + 1))
    say "FAIL [$scenario] $*" >&2
    {
        echo "--- cell log, last 40 lines (readings and answered queries left out) ---"
        cell_log | grep -vE ' reading (accepted )?refrigerator_id| (status|incidents) sent ' | tail -n 40 | cut -c1-240
        echo "--- cell log, last 5 readings ---"
        cell_log | grep -E ' reading refrigerator_id' | tail -n 5 | cut -c1-200
        echo "--- runtime warnings and errors ---"
        runtime_warnings | tail -n 15 | cut -c1-240
        echo "---"
    } >&2
}

cleanup() {
    status=$?
    # Stdout may be gone (a pipe that closed early); the cleanup must not
    # die on its own output.
    exec >&2
    if [[ "$runtime_started" == true ]]; then
        say "==> tearing down"
        for cell in "${CELLS[@]}"; do
            myrmic --timeout 5s --connect "$MYRMIC_CONNECT" delete "${SRN_PREFIX:-}${cell}" --cell >/dev/null 2>&1 || true
        done
        myrmic runtimes delete "$runtime" >/dev/null 2>&1 || true
        sleep 1
    fi
    rm -rf "$XDG_DATA_HOME"
    if [[ $status -eq 0 && $failures -gt 0 ]]; then
        status=1
    fi
    if [[ $status -eq 0 ]]; then
        say "==> e2e passed"
    else
        say "==> e2e FAILED ($failures failed assertion(s))" >&2
    fi
    exit "$status"
}
trap cleanup EXIT
# A closed pipe, Ctrl-C or SIGTERM must still reach cleanup, or a runtime is
# left running with a sensor publishing into everyone's network.
trap 'exit 1' PIPE INT TERM

# ---------------------------------------------------------------------------
# Runtime and cells
# ---------------------------------------------------------------------------

# Retries a check once per second until it passes or `deadline` seconds are
# used up. Returns 1 when it timed out.
wait_for() {
    local deadline="$1" what="$2"
    shift 2
    for _ in $(seq 1 "$deadline"); do
        if "$@"; then
            return 0
        fi
        sleep 1
    done
    say "timed out after ${deadline}s waiting for ${what}" >&2
    return 1
}

runtime_ready() {
    myrmic --timeout 2s --connect "$MYRMIC_CONNECT" network --once 2>/dev/null | grep -q -- "$runtime"
}

start_runtime() {
    say "==> starting runtime ${runtime} on ${MYRMIC_CONNECT}"
    quiet myrmic runtimes start "${CONFIG_DIR}/runtime.yml" --name "$runtime" --tag "$runtime" --detached
    runtime_started=true
    wait_for 30 "runtime ${runtime} to join the network" runtime_ready
}

cell_started() {
    cell_log | grep -q -- "$1 started"
}

# Deploys the cells under the scenario's prefix and waits for their inits.
deploy_cells() {
    export SRN_PREFIX="${run_id}-${scenario}-"
    quiet scripts/deploy.sh || { fail "deploy failed (see the error above)"; return 1; }
    for cell in "${CELLS[@]}"; do
        wait_for 20 "${cell} to start" cell_started "$cell" || fail "${cell} did not start"
    done
}

delete_cells() {
    for cell in "${CELLS[@]}"; do
        m delete "${SRN_PREFIX}${cell}" --cell >/dev/null 2>&1 || true
    done
}

sensor() { echo "${SRN_PREFIX}sensor-cell"; }
asset() { echo "${SRN_PREFIX}fridge-asset-cell"; }
monitor() { echo "${SRN_PREFIX}risk-monitor-cell"; }

# myrmic prints its progress on stderr; only failures are worth seeing. The
# command's own exit status is kept.
quiet() {
    "$@" 2>&1 >/dev/null | grep -vE '^INFO ' >&2 || true
    return "${PIPESTATUS[0]}"
}

send() {
    quiet m send "$@"
}

# ---------------------------------------------------------------------------
# The operator CLI, pointed at this run's runtime (MYRMIC_CONNECT is exported
# above) and at the current scenario's cells.
# ---------------------------------------------------------------------------

cg() {
    "$chillguard" --prefix "$SRN_PREFIX" "$@"
}

# `chillguard --json status`: {"snapshot": {...}, "open_incident": ..., ...}
get_status() {
    cg --json status
}

# `chillguard --json incidents`: {"incidents": [...], "acknowledgements": [...], ...}
get_incidents() {
    cg --json incidents --limit 20
}

# json <json document> <python expression over it, as `b`>
json() {
    python3 -I -c '
import json, sys
value = eval(sys.argv[2], {}, {"b": json.loads(sys.argv[1])})
print(json.dumps(value) if not isinstance(value, str) else value)
' "$1" "$2"
}

status_is() {
    local snapshot
    snapshot="$(get_status)" || return 1
    [[ "$(json "$snapshot" 'b["snapshot"]["status"]')" == "$1" ]]
}

# ---------------------------------------------------------------------------
# Assertions
# ---------------------------------------------------------------------------

assert_eq() {
    local what="$1" expected="$2" actual="$3"
    if [[ "$expected" == "$actual" ]]; then
        say "ok   [$scenario] ${what} = ${actual}"
    else
        fail "${what}: expected ${expected}, got ${actual}"
    fi
}

assert_log_count() {
    local what="$1" expected="$2" pattern="$3"
    local actual
    actual="$(cell_log | grep -c -- "$pattern" || true)"
    assert_eq "$what" "$expected" "$actual"
}

# Waits until the asset reports `status`, or fails.
expect_status() {
    local status="$1" deadline="$2"
    if wait_for "$deadline" "status ${status}" status_is "$status"; then
        say "ok   [$scenario] status reached ${status}"
    else
        fail "status did not reach ${status} within ${deadline}s"
    fi
}

# ---------------------------------------------------------------------------
# Scenarios
# ---------------------------------------------------------------------------

# A normal day: readings flow, nothing is raised.
scenario_normal() {
    deploy_cells
    step "letting the sensor run for 15 s on normal_day"
    sleep 15
    local snapshot incidents
    snapshot="$(get_status)" || { fail "no status snapshot"; return; }
    assert_eq "status" normal "$(json "$snapshot" 'b["snapshot"]["status"]')"
    assert_eq "refrigerator_id" "$fridge" "$(json "$snapshot" 'b["snapshot"]["refrigerator_id"]')"
    assert_eq "a reading was accepted" true "$(json "$snapshot" 'b["snapshot"]["last_reading"] is not None')"
    assert_eq "no open incident" true "$(json "$snapshot" 'b["open_incident"] is None and b["monitor_error"] is None')"
    assert_eq "last reading in range" true \
        "$(json "$snapshot" '2.0 <= b["snapshot"]["last_reading"]["temperature_c"] <= 8.0')"
    incidents="$(get_incidents)" || { fail "no incident list"; return; }
    assert_eq "incidents" 0 "$(json "$incidents" 'len(b["incidents"])')"

    step "changing a limit through the CLI, and back"
    cg set-limits --upper 9.5 >/dev/null || fail "set-limits --upper 9.5 failed"
    assert_eq "upper limit" 9.5 "$(json "$(get_status)" 'b["snapshot"]["config"]["upper_limit_c"]')"
    cg set-limits --upper 8 >/dev/null || fail "set-limits --upper 8 failed"
    assert_eq "upper limit restored" 8.0 "$(json "$(get_status)" 'b["snapshot"]["config"]["upper_limit_c"]')"
    assert_log_count "limit changes logged" 2 "limits changed refrigerator_id=${fridge} "
    if cg set-limits --lower 20 >/dev/null 2>&1; then
        fail "set-limits accepted a lower limit above the upper limit"
    else
        say "ok   [$scenario] an invalid limit is refused by the CLI"
    fi
    assert_log_count "alerts" 0 "ALERT"
    assert_log_count "status changes" 0 "status changed"
    assert_log_count "rejected readings" 0 "reading rejected"
}

# The door stands open: warning, critical after 5 s, recovery once the cabinet
# has cooled, one incident with the right record.
scenario_excursion() {
    deploy_cells
    step "starting short_door_open (door opens after 10 s, stays open 5 s)"
    cg simulate short_door_open --seed 7 >/dev/null || { fail "simulate failed"; return; }
    expect_status warning 30
    expect_status critical 20
    local incidents open
    incidents="$(get_incidents)" || { fail "no incident list"; return; }
    assert_eq "open incidents while critical" 1 "$(json "$incidents" 'len(b["incidents"])')"
    open="$(json "$incidents" 'b["incidents"][0]')"
    assert_eq "incident is open" true "$(json "$incidents" 'b["incidents"][0]["recovered_at"] is None')"
    assert_eq "incident severity" critical "$(json "$incidents" 'b["incidents"][0]["severity"]')"
    assert_eq "exposure at least critical_after" true \
        "$(json "$incidents" "b[\"incidents\"][0][\"exposure\"] >= $critical_after_ms")"
    assert_eq "initial temperature is the door-open reading" 12.0 \
        "$(json "$incidents" 'b["incidents"][0]["initial_temperature_c"]')"
    : "$open"

    expect_status normal 60
    incidents="$(get_incidents)" || { fail "no incident list"; return; }
    assert_eq "incidents after recovery" 1 "$(json "$incidents" 'len(b["incidents"])')"
    assert_eq "incident closed" true "$(json "$incidents" 'b["incidents"][0]["recovered_at"] is not None')"
    assert_eq "closed incident keeps critical" critical "$(json "$incidents" 'b["incidents"][0]["severity"]')"
    assert_eq "recovery temperature inside the hysteresis band" true \
        "$(json "$incidents" 'b["incidents"][0]["recovery_temperature_c"] <= 7.5')"
    assert_eq "no data gap" false "$(json "$incidents" 'b["incidents"][0]["has_data_gap"]')"
    assert_log_count "warning alerts" 1 "ALERT severity=warning"
    assert_log_count "critical alerts" 1 "ALERT severity=critical"
    assert_log_count "recoveries" 1 "recovered refrigerator_id"
    assert_log_count "status changes" 3 "status changed refrigerator_id"

    step "acknowledging the incident through the CLI"
    local id
    id="$(json "$incidents" 'b["incidents"][0]["incident_id"]')"
    cg ack "$id" >/dev/null || fail "ack ${id} failed"
    incidents="$(get_incidents)" || { fail "no incident list"; return; }
    assert_eq "acknowledged incident" "$id" "$(json "$incidents" 'b["acknowledgements"][0]["incident_id"]')"
    cg ack "$id" >/dev/null || fail "a second ack ${id} failed"
    assert_log_count "acknowledgements recorded" 1 "incident acknowledged refrigerator_id=${fridge} incident_id=${id} "
    if cg ack "${fridge}-1" >/dev/null 2>&1; then
        fail "ack accepted an unknown incident"
    else
        say "ok   [$scenario] an unknown incident is refused by the CLI"
    fi
}

# The sensor stops: the monitor's tick declares it stale within stale_after
# plus one tick; a returning sensor clears it.
scenario_stale() {
    deploy_cells
    sleep 3
    step "deleting the sensor cell"
    quiet m delete "$(sensor)" --cell
    local deleted_at
    deleted_at="$(date +%s)"
    expect_status sensor_stale 30
    local took=$(( $(date +%s) - deleted_at ))
    # Noticed by the first tick more than stale_after after the last reading:
    # at most stale_after + one tick (stale_after / 2) + a second of slack.
    assert_eq "stale noticed within 16 s (took ${took}s)" 1 "$(( took <= 16 ))"
    assert_eq "stale noticed only after stale_after (took ${took}s)" 1 "$(( took >= stale_after_ms / 1000 ))"
    assert_log_count "stale warnings" 1 "sensor stale refrigerator_id"
    assert_log_count "alerts" 0 "ALERT"
    step "redeploying the sensor cell"
    quiet scripts/deploy.sh sensor-cell || { fail "deploy failed (see the error above)"; return 1; }
    expect_status normal 15
    assert_log_count "sensor resumed" 1 "sensor resumed"
    assert_log_count "status changes" 2 "status changed refrigerator_id"
}

# The runtime goes down during an excursion. Afterwards: same incident, no
# second alert, exposure grew across the outage and the gap is recorded.
scenario_restart() {
    deploy_cells
    step "starting power_failure_2h (power fails after 30 s, warms past 8 °C about 60 s later)"
    cg simulate power_failure_2h --seed 3 >/dev/null || { fail "simulate failed"; return; }
    expect_status warning 150
    expect_status critical 20
    local before incident_id exposure_before
    before="$(get_incidents)" || { fail "no incident list"; return; }
    incident_id="$(json "$before" 'b["incidents"][0]["incident_id"]')"
    exposure_before="$(json "$before" 'b["incidents"][0]["exposure"]')"

    step "stopping the runtime mid-incident (incident ${incident_id}, exposure ${exposure_before} ms)"
    local down_at
    down_at="$(date +%s)"
    quiet myrmic runtimes delete "$runtime"
    # Longer than stale_after, so that the silence counts as a data gap
    # (docs/decisions/0003-risk-policies.md, policy 6).
    local down_for=$(( stale_after_ms / 1000 + 5 ))
    step "runtime down for ${down_for} s"
    sleep "$down_for"
    start_runtime
    step "redeploying the cells with the same payloads"
    deploy_cells
    local outage=$(( $(date +%s) - down_at ))
    say "     outage lasted about ${outage}s"
    # The monitor's init must find the open incident, not raise it again.
    assert_log_count "monitor resumed critical" 1 "risk-monitor-cell started .*resumed=true status=critical incident_id=${incident_id}"
    assert_log_count "asset resumed" 1 "fridge-asset-cell started .*resumed=true"
    sleep 3
    local after
    after="$(get_incidents)" || { fail "no incident list"; return; }
    assert_eq "still one incident" 1 "$(json "$after" 'len(b["incidents"])')"
    assert_eq "same incident" "$incident_id" "$(json "$after" 'b["incidents"][0]["incident_id"]')"
    assert_eq "incident still open" true "$(json "$after" 'b["incidents"][0]["recovered_at"] is None')"
    # Exposure counts the silence in full: at least the time the runtime
    # was down, less the second a reading may have been late by.
    assert_eq "exposure grew by at least the outage" true \
        "$(json "$after" "b[\"incidents\"][0][\"exposure\"] >= $exposure_before + ${down_for}000 - 1000")"
    assert_eq "outage recorded as a data gap" true "$(json "$after" 'b["incidents"][0]["has_data_gap"]')"
    assert_eq "status after restart" critical "$(json "$(get_status)" 'b["snapshot"]["status"]')"
    assert_eq "status reports the open incident" "$incident_id" \
        "$(json "$(get_status)" 'b["open_incident"]["incident_id"]')"
    assert_log_count "warning alerts (whole scenario)" 1 "ALERT severity=warning"
    assert_log_count "critical alerts (whole scenario)" 1 "ALERT severity=critical"

    expect_status normal 150
    after="$(get_incidents)" || { fail "no incident list"; return; }
    assert_eq "incident recovered" true "$(json "$after" 'b["incidents"][0]["recovered_at"] is not None')"
    assert_eq "recovered incident keeps critical" critical "$(json "$after" 'b["incidents"][0]["severity"]')"
    assert_log_count "recoveries" 1 "recovered refrigerator_id"
    assert_log_count "warning alerts (whole scenario)" 1 "ALERT severity=warning"
    assert_log_count "critical alerts (whole scenario)" 1 "ALERT severity=critical"
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

scenarios=("$@")
if [[ ${#scenarios[@]} -eq 0 ]]; then
    scenarios=(normal excursion stale restart)
fi
for s in "${scenarios[@]}"; do
    declare -F "scenario_$s" >/dev/null || { echo "unknown scenario: $s" >&2; exit 2; }
done

# The operator CLI is how this harness talks to the cells; build it first.
cargo build --quiet -p chillguard-cli
chillguard="target/debug/chillguard"

# The same checks deploy.sh makes, before a runtime is started for nothing:
# every binary exists and is newer than the sources.
for cell in "${CELLS[@]}"; do
    wasm="$(wasm_path "$cell")"
    [[ -f "$wasm" ]] || { echo "error: $wasm not found; run 'make build-cells' first" >&2; exit 1; }
    stale="$(stale_source "$cell" "$wasm")"
    [[ -z "$stale" ]] || { echo "error: $wasm is older than $stale; run 'make build-cells' first" >&2; exit 1; }
done

start_runtime
for scenario in "${scenarios[@]}"; do
    scenario_since="$(date -u +%Y-%m-%dT%H:%M:%S)"
    say "==> scenario: $scenario"
    "scenario_$scenario" || say "==> scenario $scenario stopped early"
    # Always, or a scenario that stopped early leaves a sensor publishing
    # readings under this run's refrigerator id into the next scenario.
    delete_cells
done
