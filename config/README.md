# Example configuration

| File | What it is | Status |
|---|---|---|
| `runtime.dev.yml` | Myrmic runtime configuration for a single-node dev setup. | Used by `make e2e`. The runtime rejects unknown keys, so every key in it is one the installed runtime accepts. |
| `refrigerator.example.json` | Per-refrigerator settings, shaped as the init payload a cell would receive through `myrmic deploy --init-file`. | Provisional. No cell reads it yet, and the field names follow docs/architecture.md and may change when the domain types are written. |

Start a dev runtime with:

```sh
myrmic runtimes start config/runtime.dev.yml --name chillguard-dev
```
