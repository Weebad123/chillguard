# ChillGuard task runner. `make help` lists the targets.

CELLS    := sensor-cell fridge-asset-cell risk-monitor-cell
CELL_DIR := crates/cells
# Init payloads for `make deploy`: `prod` (default) or `dev` (60x clock,
# `simulate` accepted). See config/README.md.
PROFILE  ?= prod
export PROFILE
# Line coverage is measured over every host test and reported for
# chillguard-core's non-test sources only.
COV_MIN    := 90
COV_IGNORE := (crates/(chillguard-protocol|chillguard-cli|cells)/|crates/chillguard-core/tests/|/tests\.rs$$)

.DEFAULT_GOAL := help
.PHONY: help fmt fmt-check lint test doc deny coverage build-cells cli deploy e2e ci

help: ## List the targets
	@grep -E '^[a-z0-9-]+:.*## ' $(MAKEFILE_LIST) | awk -F ':.*## ' '{printf "  %-12s %s\n", $$1, $$2}'

fmt: ## Format every crate
	cargo fmt --all

fmt-check: ## Check formatting without changing files
	cargo fmt --all --check

lint: ## Clippy on every crate and target, warnings are errors
	cargo clippy --workspace --all-targets -- -D warnings

test: ## Run the host tests
	cargo test --workspace --locked

doc: ## Build the API docs, warnings are errors
	RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked

deny: ## Check dependency licences and security advisories (needs cargo-deny)
	cargo deny check

coverage: ## Line coverage of chillguard-core from every host test; fails below $(COV_MIN) % (needs cargo-llvm-cov)
	cargo llvm-cov --workspace --locked --no-report
	cargo llvm-cov report --ignore-filename-regex '$(COV_IGNORE)' --fail-under-lines $(COV_MIN)
	@mkdir -p target/llvm-cov
	cargo llvm-cov report --ignore-filename-regex '$(COV_IGNORE)' --lcov --output-path target/llvm-cov/lcov.info

# `myrmic build` on the workspace root fails on the non-cell crates, so each
# cell crate is built on its own (see docs/myrmic-notes.md).
build-cells: ## Build every cell to WebAssembly
	@set -e; for cell in $(CELLS); do \
		echo "==> myrmic build $(CELL_DIR)/$$cell"; \
		myrmic build $(CELL_DIR)/$$cell; \
	done

cli: ## Build the operator CLI (target/debug/chillguard)
	cargo build -p chillguard-cli

deploy: build-cells ## Build the cells and deploy them to a running runtime (PROFILE=prod|dev)
	scripts/deploy.sh

e2e: build-cells cli ## Run the end-to-end scenarios on a throwaway runtime (about 5 min)
	scripts/e2e.sh

ci: fmt-check lint test doc deny coverage build-cells ## Run every quality gate (what .github/workflows/ci.yml runs)
