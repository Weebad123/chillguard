#!/usr/bin/env bash
# End-to-end test harness: a throwaway Myrmic runtime, every cell, and the
# simulator's scenarios played at 60x (config/e2e/).
#
# Usage: scripts/e2e.sh [scenario...]       (run `make build-cells` first)
#   scenarios: normal excursion stale restart dashboard playground    (default: all)
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
#
# The runtime's embedded gateway serves the web dashboard on a port of this
# run. Unlike the swarm side it listens on every interface, so the dashboard
# cell is deployed without the gateway's message API (`dev_controls`) except
# for the part of the `dashboard` scenario that tests it.
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
gateway_port=$(( 30000 + $$ % 10000 ))
dashboard_url="http://127.0.0.1:${gateway_port}/chillguard"
# config/e2e/*.json name the refrigerator `fridge-e2e` and runtime.yml the
# ports `E2E_PORT` and `E2E_GATEWAY_PORT`; this run's copies name them after
# the run. The dashboard's payload is written per scenario (dashboard_config).
CONFIG_DIR="${XDG_DATA_HOME}/config"
mkdir -p "$CONFIG_DIR"
for file in config/e2e/*.json; do
    sed "s/\"fridge-e2e\"/\"${fridge}\"/" "$file" > "${CONFIG_DIR}/$(basename "$file")"
done
sed -e "s/E2E_PORT/${port}/" -e "s/E2E_GATEWAY_PORT/${gateway_port}/" \
    config/e2e/runtime.yml > "${CONFIG_DIR}/runtime.yml"
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
playground=""             # the runtime `chillguard start` made, while it is up
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
    if [[ -n "$playground" ]]; then
        "$chillguard" stop --name "$playground" >/dev/null 2>&1 || true
    fi
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

# Writes the dashboard cell's payload for the current scenario: it names the
# other cells, whose names carry the scenario's prefix.
#
# Usage: dashboard_config <dev_controls: true|false>
dashboard_config() {
    sed -e "s/E2E_PREFIX/${SRN_PREFIX}/g" -e "s/\"dev_controls\": false/\"dev_controls\": $1/" \
        config/e2e/dashboard-cell.dev.json > "${CONFIG_DIR}/dashboard-cell.dev.json"
}

# Deploys the cells under the scenario's prefix and waits for their inits.
deploy_cells() {
    export SRN_PREFIX="${run_id}-${scenario}-"
    dashboard_config false
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
# The web dashboard, through the runtime's embedded gateway.
# ---------------------------------------------------------------------------

# Prints the HTTP status of a request; `000` when nothing answered.
http_code() {
    curl -s -o /dev/null -m 5 -w '%{http_code}' "$@" || true
}

http_ok() {
    [[ "$(http_code "$1")" == 200 ]]
}

# A file the dashboard cell serves, or nothing.
served() {
    curl -fsS -m 5 "${dashboard_url}/$1" 2>/dev/null
}

# The status in the asset cell's answer, as the page reads it.
served_status_is() {
    local answer
    answer="$(served status/0.json)" || return 1
    [[ "$(json "$answer" 'b["body"]["status"]')" == "$1" ]]
}

# The newest incident's severity in the risk monitor's answer.
served_incident_is() {
    local answer
    answer="$(served incidents/0.json)" || return 1
    [[ "$(json "$answer" 'b["body"]["incidents"][0]["severity"] if b["body"]["incidents"] else None')" == "$1" ]]
}

page_offers_controls() {
    local config
    config="$(served config.json)" || return 1
    [[ "$(json "$config" 'b["dev_controls"]')" == true ]]
}

# Sends `simulate_scenario` to the sensor the way the page's buttons do: a
# GET on the API path opens a session, whose id a POST must present, with the
# payload as base64. Prints the HTTP status of the POST.
gateway_simulate() {
    local preset="$1" stream="${XDG_DATA_HOME}/gateway-stream" session payload reader
    curl -sS -N -m 20 "${dashboard_url}/api" > "$stream" 2>/dev/null &
    reader=$!
    wait_for 10 "a gateway session" grep -q '"session"' "$stream" || true
    session="$(sed -n 's/.*"session":"\([^"]*\)".*/\1/p' "$stream" | head -n 1)"
    payload="$(printf '{"v":1,"body":{"preset":"%s"}}' "$preset" | base64 -w0)"
    http_code -X POST "${dashboard_url}/api" \
        -H 'Content-Type: application/json' -H "X-Gateway-Session: ${session}" \
        -d "{\"type\":\"command\",\"sri\":\"$(sensor)\",\"name\":\"simulate_scenario\",\"payload\":\"${payload}\"}"
    kill "$reader" 2>/dev/null || true
    wait "$reader" 2>/dev/null || true
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

# The web dashboard: the gateway serves the page and the cells' answers
# beside it; the page's scenario buttons work only with dev_controls.
scenario_dashboard() {
    deploy_cells
    step "fetching ${dashboard_url}/"
    if wait_for 20 "the dashboard route" http_ok "${dashboard_url}/"; then
        say "ok   [$scenario] the dashboard route responds"
    else
        fail "GET ${dashboard_url}/ answered $(http_code "${dashboard_url}/"), not 200"
        return
    fi
    assert_eq "the route serves the page" 1 "$(served "" | grep -c '<title>ChillGuard</title>' || true)"
    wait_for 15 "the asset cell's answer" http_ok "${dashboard_url}/status/0.json" || true
    wait_for 15 "the risk monitor's answer" http_ok "${dashboard_url}/incidents/0.json" || true
    local answer
    answer="$(served status/0.json)" || { fail "status/0.json is not served"; return; }
    assert_eq "served status is this refrigerator's" "$fridge" "$(json "$answer" 'b["body"]["refrigerator_id"]')"
    assert_eq "served status" normal "$(json "$answer" 'b["body"]["status"]')"
    answer="$(served incidents/0.json)" || { fail "incidents/0.json is not served"; return; }
    assert_eq "served incidents" 0 "$(json "$answer" 'len(b["body"]["incidents"])')"
    assert_eq "an unknown path" 404 "$(http_code "${dashboard_url}/status/1.json")"
    # Without dev_controls there is no way from HTTP into the swarm.
    assert_eq "message API without dev_controls" 404 "$(http_code "${dashboard_url}/api")"

    step "redeploying the dashboard cell with dev_controls"
    dashboard_config true
    quiet scripts/deploy.sh dashboard-cell || { fail "deploy failed (see the error above)"; return; }
    wait_for 20 "the page to offer the controls" page_offers_controls || fail "config.json does not say dev_controls"
    step "starting short_door_open through the gateway's message API"
    assert_eq "simulate_scenario accepted by the gateway" 202 "$(gateway_simulate short_door_open)"
    if wait_for 30 "the served status to reach critical" served_status_is critical; then
        say "ok   [$scenario] the served status reached critical"
    else
        fail "the served status did not reach critical within 30s"
    fi
    assert_log_count "scenario started by the page's command" 1 "scenario started refrigerator_id=${fridge} preset=short_door_open"
    # The two answers are stored one after the other; give the second a moment.
    wait_for 5 "the served incident to reach critical" served_incident_is critical || true
    answer="$(served incidents/0.json)" || { fail "incidents/0.json is not served"; return; }
    assert_eq "served incidents during the excursion" 1 "$(json "$answer" 'len(b["body"]["incidents"])')"
    assert_eq "served incident severity" critical "$(json "$answer" 'b["body"]["incidents"][0]["severity"]')"
    assert_eq "served incident is open" true "$(json "$answer" 'b["body"]["incidents"][0]["recovered_at"] is None')"
    assert_eq "served exposure at least critical_after" true \
        "$(json "$answer" "b[\"body\"][\"incidents\"][0][\"exposure\"] >= $critical_after_ms")"
    assert_log_count "answers the dashboard refused" 0 "answer ignored"
}

# `chillguard start` and `stop`: the playground, which is a runtime of its
# own beside this run's, under a name and on ports of this run.
scenario_playground() {
    # Nothing is deployed under this prefix; the teardown expects one.
    export SRN_PREFIX="${run_id}-${scenario}-"
    local name="play-${run_id}" page="http://127.0.0.1:$(( 40000 + $$ % 10000 ))/chillguard"
    local -a start=("$chillguard" --json start --no-open --name "$name"
        --port "$(( 40000 + $$ % 10000 ))" --swarm-port "$(( 50000 + $$ % 10000 ))")
    local started snapshot

    step "chillguard start"
    playground="$name"
    started="$("${start[@]}" 2>/dev/null)" || { fail "chillguard start failed: $("${start[@]}" 2>&1 | tail -n 1)"; return; }
    assert_eq "start started the playground" false "$(json "$started" 'b["already_running"]')"
    assert_eq "start names the dashboard" "http://localhost:$(( 40000 + $$ % 10000 ))/chillguard/" "$(json "$started" 'b["url"]')"
    # start returns once the page shows the refrigerator: no waiting here.
    assert_eq "the dashboard responds" 200 "$(http_code "${page}/")"
    assert_eq "the dashboard shows the refrigerator" 200 "$(http_code "${page}/status/0.json")"
    assert_eq "the dashboard offers the scenarios" true "$(json "$(curl -fsS -m 5 "${page}/config.json")" 'b["dev_controls"]')"
    # The endpoint start prints is how the rest of the CLI reaches it.
    snapshot="$("$chillguard" --json --connect "$(json "$started" 'b["connect"]')" status)" || { fail "no status from the playground"; return; }
    assert_eq "status through the endpoint start printed" normal "$(json "$snapshot" 'b["snapshot"]["status"]')"

    step "chillguard start again"
    started="$("${start[@]}" 2>/dev/null)" || { fail "a second chillguard start failed"; return; }
    assert_eq "the second start found it running" true "$(json "$started" 'b["already_running"]')"
    assert_eq "the dashboard still responds" 200 "$(http_code "${page}/")"

    step "chillguard stop"
    assert_eq "stop stopped it" true "$(json "$("$chillguard" --json stop --name "$name")" 'b["was_running"]')"
    playground=""
    assert_eq "the dashboard is gone" 000 "$(http_code "${page}/")"
    assert_eq "a second stop finds nothing" false "$(json "$("$chillguard" --json stop --name "$name")" 'b["was_running"]')"
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

scenarios=("$@")
if [[ ${#scenarios[@]} -eq 0 ]]; then
    scenarios=(normal excursion stale restart dashboard playground)
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
