# Shared by the deploy and e2e scripts: the list of cells and where their
# Wasm binaries land. Sourced, not executed.

# Keep in sync with CELLS in the Makefile.
CELLS=(sensor-cell fridge-asset-cell risk-monitor-cell)

# Print the path of the Wasm binary `myrmic build` produces for a cell crate.
wasm_path() {
    echo "target/wasm32-unknown-unknown/release/${1//-/_}.wasm"
}

# Print the first source file of a cell that is newer than its binary, or
# nothing when the binary is current. A binary older than the sources it was
# built from is not what the tree says it is (seen once: a build reported
# `Finished` and left an older artifact in place, and the deploy then failed
# in init). A cell is built from its own crate, chillguard-core and
# chillguard-protocol; other cells and the operator CLI are not part of it,
# and neither are test-only sources (`tests/` directories, `tests.rs`
# modules), or cargo's correct "nothing to do" would be refused here.
#
# Usage: stale_source <cell> <wasm path>
stale_source() {
    find "crates/cells/$1" crates/chillguard-core crates/chillguard-protocol \
        -name '*.rs' -not -path '*/tests/*' -not -name tests.rs \
        -newer "$2" -print -quit
}
