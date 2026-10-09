# Shared by the deploy and e2e scripts: the list of cells and where their
# Wasm binaries land. Sourced, not executed.

# Keep in sync with CELLS in the Makefile.
CELLS=(sensor-cell fridge-asset-cell risk-monitor-cell)

# Print the path of the Wasm binary `myrmic build` produces for a cell crate.
wasm_path() {
    echo "target/wasm32-unknown-unknown/release/${1//-/_}.wasm"
}
