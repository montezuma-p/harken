.PHONY: build test fmt fmt-fix lint audit deps check clean install-dev-tools bench demo demo-cli demo-claude

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

# Records the two README GIFs (needs asciinema, agg, ffmpeg, espeak-ng/piper;
# demo-claude also needs `claude`, and its take is manual). 'demo' requires
# ZIP=<validated export.zip>; the single-GIF targets generate a fresh, unchecked
# export when ZIP is omitted.
demo:
ifndef ZIP
	$(error recording both GIFs needs ZIP=<validated export.zip>, or the two \
	takes end up with different audio. Generate one with \
	scripts/demo/make-demo-zip.sh and spot-check the transcripts first)
endif
	$(MAKE) demo-claude ZIP=$(ZIP)
	$(MAKE) demo-cli ZIP=$(ZIP)

demo-cli:
	cargo build --release
	bash scripts/demo/record-demo.sh $(ZIP)

demo-claude:
	cargo build --release
	bash scripts/demo/record-claude-demo.sh $(ZIP)
