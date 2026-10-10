# End-to-end test configuration

Init payloads and the runtime configuration for `scripts/e2e.sh`. The
payloads have the same shape as the files in `config/`, with the clock
compressed so the whole suite runs in a few minutes:

| Setting | Value | Effect |
|---|---|---|
| sensor `time_scale` | 60 | one real second plays one scenario minute; `short_door_open` takes 90 s, `power_failure_2h` 5 min |
| sensor `interval_ms` | 1000 | one reading a second |
| asset `critical_after` | 5 s | an excursion is critical after five readings |
| asset `stale_after` | 10 s | the monitor ticks every 5 s and declares the sensor stale after 10 s of silence |
| asset `hysteresis_c` | 0.5 | noise of ±0.2 °C around a limit does not reopen an incident |

Readings keep real timestamps; only the thermal model runs fast.

The files name the refrigerator `fridge-e2e`. The harness does not deploy
them as they are: it writes copies with a refrigerator id unique to the run
(`fridge-e2e-<pid>`), because events travel the whole network and two runs
sharing an id would feed each other's readings.

`runtime.yml` disables multicast scouting and listens on loopback only, so
the test runtime is invisible to other runtimes on the machine or network
and is driven with `--connect tcp/127.0.0.1:<port>`. The harness writes a
copy with `E2E_PORT` replaced by a port derived from its process id.
