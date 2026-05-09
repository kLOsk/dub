.DEFAULT_GOAL := help

.PHONY: help fmt fmt-check clippy test smoke rt-audit cov fuzz-quick soak clean ci

help:
	@echo "Dub — common targets"
	@echo "  make test          run all tests (cargo nextest + clippy)"
	@echo "  make smoke         run the dub-cli smoke binary"
	@echo "  make rt-audit      run the RT-safety harness"
	@echo "  make fmt           cargo fmt"
	@echo "  make fmt-check     cargo fmt --check"
	@echo "  make clippy        cargo clippy --all-targets -- -D warnings"
	@echo "  make cov           coverage report (requires cargo-llvm-cov)"
	@echo "  make fuzz-quick    run fuzz targets for 60s each (placeholder)"
	@echo "  make soak          1-hour offline render soak (placeholder)"
	@echo "  make ci            run the full CI pipeline locally"
	@echo "  make clean         cargo clean"

fmt:
	cargo fmt

fmt-check:
	cargo fmt --check

clippy:
	cargo clippy --all-targets --workspace -- -D warnings

# Prefer nextest if installed; fall back to cargo test.
test: clippy
	@if command -v cargo-nextest >/dev/null 2>&1; then \
		cargo nextest run --workspace; \
	else \
		echo "[hint] install cargo-nextest for faster runs: cargo install cargo-nextest --locked"; \
		cargo test --workspace; \
	fi

smoke:
	cargo run -p dub-cli -- smoke

rt-audit:
	cargo run -p dub-cli -- rt-audit

cov:
	@command -v cargo-llvm-cov >/dev/null 2>&1 || { \
		echo "cargo-llvm-cov not installed. Install: cargo install cargo-llvm-cov --locked"; exit 1; }
	cargo llvm-cov --workspace --html --output-dir coverage

fuzz-quick:
	@echo "[placeholder] fuzz targets are added per parser as they land. See PRD §2.2.5."

soak:
	@echo "[placeholder] soak harness lands in M2. See PRD §2.2.0 phase B."

ci: fmt-check clippy test
	@echo "Local CI pipeline complete."

clean:
	cargo clean
