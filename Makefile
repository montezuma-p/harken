.PHONY: build test fmt fmt-fix lint audit deps check clean install-dev-tools bench demo

build:
	cargo build

test:
	cargo test

fmt:
	cargo fmt --check

fmt-fix:
	cargo fmt

lint:
	cargo clippy --all-targets -- -D warnings

# Security advisories against RustSec (needs cargo-audit: make install-dev-tools)
audit:
	cargo audit

# Unused dependency check (needs cargo-machete: make install-dev-tools)
deps:
	cargo machete

check: fmt lint test audit deps

install-dev-tools:
	cargo install cargo-audit cargo-machete

clean:
	cargo clean

# Slow, needs models + a release build — never part of 'check'.
# AUDIO=<file> to bench your own recording; defaults to synthetic TTS audio.
bench:
	cargo build --release
	bash scripts/bench/bench.sh $(or $(AUDIO),--synth)

# Records docs/assets/demo.gif (needs asciinema, agg, ffmpeg, espeak-ng/piper).
demo:
	cargo build --release
	bash scripts/demo/record-demo.sh
