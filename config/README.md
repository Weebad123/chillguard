# Example configuration

| File | What it is | Status |
|---|---|---|
| `runtime.dev.yml` | Myrmic runtime configuration for a single-node dev setup. | Used by `make e2e`. The runtime rejects unknown top-level keys only; a misspelt key inside a section is ignored silently. |
| `sensor-cell.dev.json` | Init payload of the sensor cell: refrigerator, reading interval in milliseconds, `dev_mode`, noise seed. | Used by `make deploy` and `make e2e` (`scripts/deploy.sh` passes `config/<cell>.dev.json` as `--init-file`). The cell refuses unknown keys. |
| `fridge-asset-cell.dev.json` | Init payload of the asset cell: the refrigerator's configuration (durations in milliseconds) and `record_history`, which turns on the time-series history. | Used by `make deploy` and `make e2e`. The configuration only applies to a first deployment: a cell that already holds state for the refrigerator keeps its stored configuration; change it with the `set_limits` command. |
| `risk-monitor-cell.dev.json` | Init payload of the risk monitor cell: the refrigerator it evaluates. Limits are not in it; they arrive with every `reading_accepted` from the asset cell. | Used by `make deploy` and `make e2e`. |

`e2e/` holds the same payloads with a compressed clock for `scripts/e2e.sh`, plus a
runtime configuration that keeps the test runtime off the network; see `e2e/README.md`.

Start a dev runtime with:

```sh
myrmic runtimes start config/runtime.dev.yml --name chillguard-dev
```
