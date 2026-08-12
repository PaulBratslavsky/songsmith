# Songsmith Studio — common tasks.
.PHONY: install dev build dmg test analyzertest analyzercheck types

# Build and (re)install the app into /Applications, then launch it.
install:
	./scripts/install.sh

# Run the app in development (hot-reloads the frontend).
dev:
	cd app/src-tauri && ../../frontend/node_modules/.bin/tauri dev

# Build the .app bundle only (no installer).
build:
	cd app/src-tauri && ../../frontend/node_modules/.bin/tauri build --bundles app

# Build a distributable .dmg (Finder-scripted; can be flaky in automation).
dmg:
	cargo build -p mcp-shim --release
	cd app/src-tauri && ../../frontend/node_modules/.bin/tauri build

# Run the test suites (Rust contracts + the analyzer's pure logic).
test: analyzertest
	cargo test -p song_core

# The analyzer's pure-logic tests (stdlib unittest, no audio needed — the
# render corpus it was calibrated on isn't checked in).
analyzertest:
	cd analysis && .venv/bin/python -m unittest discover -s . -q
	analysis/.venv/bin/python analysis/eval.py --selftest

# Score the analyzer against the ground-truth corpus in analysis/bench/tracks.
# Deterministic and offline — no Claude. Run it on every analyze.py change.
analyzercheck:
	@test -f analysis/bench/tracks/_fixture-amfcg/audio.wav || analysis/.venv/bin/python analysis/make_fixture.py
	analysis/.venv/bin/python analysis/eval.py $(ARGS)

# Regenerate the TypeScript types from the Rust models.
types:
	cargo test -p song_core export_bindings

# Tier B flowcheck: walk a scratch song through the whole flow with REAL
# Claude (subscription auth), then run deterministic coherence checks.
# Several minutes; needs a logged-in `claude` CLI. See scripts/flowcheck.py.
flowcheck:
	cargo build -p mcp-shim
	python3 scripts/flowcheck.py
