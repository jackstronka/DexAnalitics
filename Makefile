# Makefile for common tasks in a Rust project
# Detect current branch
CURRENT_BRANCH := $(shell git rev-parse --abbrev-ref HEAD)
ZIP_NAME = OrderBook-rs.zip


# Default target
.PHONY: all
all: test fmt lint build

# Build the project
.PHONY: build
build:
	cargo build

.PHONY: release
release:
	cargo build --release

# Release binary for mainnet Orca bot CLI only (fast iteration)
.PHONY: cli-release
cli-release:
	cargo build --release -p clmm-lp-cli

# Recompute fee_payer_token_deltas from RPC tx meta → writes `<ledger>.enriched.jsonl` (see CLMM_POSITION_LIFECYCLE_USE_ENRICHED).
.PHONY: enrich-lifecycle-ledger
enrich-lifecycle-ledger:
	cargo run -p clmm-lp-cli --bin enrich_lifecycle_ledger --release

# Run tests (E2: cargo-nextest; install: cargo install --locked cargo-nextest)
.PHONY: test
test:
	@cargo nextest --version >/dev/null 2>&1 || (echo "E2: install cargo-nextest: cargo install --locked cargo-nextest" >&2; exit 1)
	LOGLEVEL=WARN cargo nextest run --workspace

# Format the code
.PHONY: fmt
fmt:
	cargo +stable fmt --all

# Check formatting
.PHONY: fmt-check
fmt-check:
	cargo +stable fmt --all --check

# Run Clippy for linting
.PHONY: lint
lint:
	cargo clippy --all-targets --all-features -- -D warnings

.PHONY: lint-fix
lint-fix:
	cargo clippy --fix --all-targets --all-features --allow-dirty --allow-staged -- -D warnings

# Clean the project
.PHONY: clean
clean:
	cargo clean

# Pre-push checks
.PHONY: check
check: test fmt-check lint

# Same checks as CI; run by .githooks/pre-push (Windows: tools/verify.ps1)
.PHONY: verify verify-rust verify-web test-report hooks
verify: verify-rust verify-web test-report

verify-rust: fmt-check lint
	@cargo nextest --version >/dev/null 2>&1 || (echo "E2: install cargo-nextest: cargo install --locked cargo-nextest" >&2; exit 1)
	mkdir -p target/test-report
	LOGLEVEL=WARN cargo nextest run --workspace

verify-web:
	mkdir -p target/test-report
	cd web && npm run check:api-gen && npx tsc --noEmit \
		&& npx vitest run --reporter=default --reporter=junit --outputFile=../target/test-report/vitest-junit.xml \
		&& npm run lint

# R2: per-area table from nextest + vitest JUnit (printed after verify steps)
test-report:
	@mkdir -p target/test-report
	@set -e; \
	args=""; \
	if [ -f target/nextest/default/junit.xml ]; then args="$$args --junit target/nextest/default/junit.xml"; fi; \
	if [ -f target/nextest/ci/junit.xml ]; then args="$$args --junit target/nextest/ci/junit.xml"; fi; \
	if [ -f target/test-report/vitest-junit.xml ]; then args="$$args --junit target/test-report/vitest-junit.xml"; fi; \
	if [ -z "$$args" ]; then echo "test-report: no junit xml (run verify-rust / verify-web first)"; exit 0; fi; \
	python3 tools/test_report.py $$args --out-dir target/test-report; \
	echo ""; echo "=== test report (R2) ==="; cat target/test-report/report.md

# Enable repo git hooks for this clone
hooks:
	git config core.hooksPath .githooks

# Human-readable golden .snap delta vs origin/main (R3)
.PHONY: golden-delta
golden-delta:
	python3 tools/golden_delta.py --git-base origin/main

# Rewrite crates/api/openapi.json from live utoipa spec (C1)
.PHONY: openapi
openapi:
	UPDATE_OPENAPI=1 cargo test -p clmm-lp-api --lib openapi_matches_committed

# Rewrite web/src/lib/api.gen.ts from committed OpenAPI snapshot (C2)
.PHONY: openapi-ts
openapi-ts:
	cd web && npm run gen:api

# C5: high/critical BUGS.md Guards/tests names exist in the repo
.PHONY: bugs-test-guard
bugs-test-guard:
	python3 tools/bugs_test_guard.py

# R1: test id → area map (ci/test-areas.toml) + classifier unit tests
.PHONY: test-areas
test-areas:
	python3 tools/test_areas.py --check-map
	cd tools && python3 test_test_areas.py

# R2 unit tests (hermetic fixtures)
.PHONY: test-report-unit
test-report-unit:
	cd tools && python3 test_test_report.py

# R4: weekly trend from local report.json dir (or unit fixtures)
.PHONY: test-trend test-trend-unit
test-trend-unit:
	cd tools && python3 test_test_trend.py

test-trend:
	python3 tools/test_trend.py --reports-dir tools/fixtures/r4_reports --out-dir target/test-trend
	@echo ""; echo "=== test trend (R4) ==="; cat target/test-trend/trend.md

# R5: MASTER §2.2 Ochrona testami from report.json (+ unit / --check-master)
.PHONY: test-protection test-protection-unit
REPORT ?= tools/fixtures/r5_report/report.json
test-protection-unit:
	cd tools && python3 test_test_protection.py

test-protection:
	python3 tools/test_protection.py --report $(REPORT)
	python3 tools/test_protection.py --report $(REPORT) --check-master

# Run the project
.PHONY: run
run:
	cargo run

# Web dashboard + API in one terminal (see web/scripts/start-dev-stack.mjs)
.PHONY: dashboard
dashboard:
	cd web && npm install && npm start

# API + Vite w Dockerze (patrz doc/DOCKER.md)
.PHONY: docker-up
docker-up:
	docker compose up --build

.PHONY: fix
fix:
	cargo fix --allow-staged --allow-dirty

.PHONY: pre-push
pre-push: fix fmt lint-fix test doc

.PHONY: doc
doc:
	cargo clippy -- -W missing-docs

.PHONY: doc-open
doc-open:
	cargo doc --open

.PHONY: publish
publish:
	find . -name ".DS_Store" -type f -delete | true
	# Login is handled by user session usually, or env var
	# cargo login ${CARGO_REGISTRY_TOKEN}
	cargo publish -p clmm-lp-domain
	cargo publish -p clmm-lp-simulation
	cargo publish -p clmm-lp-protocols
	cargo publish -p clmm-lp-optimization
	cargo publish -p clmm-lp-execution
	cargo publish -p clmm-lp-data
	cargo publish -p clmm-lp-api
	cargo publish -p clmm-lp-cli

.PHONY: publish-dry-run
publish-dry-run:
	cargo publish -p clmm-lp-domain --dry-run
	cargo publish -p clmm-lp-simulation --dry-run
	cargo publish -p clmm-lp-protocols --dry-run
	cargo publish -p clmm-lp-optimization --dry-run
	cargo publish -p clmm-lp-execution --dry-run
	cargo publish -p clmm-lp-data --dry-run
	cargo publish -p clmm-lp-api --dry-run
	cargo publish -p clmm-lp-cli --dry-run

# Run the project
.PHONY: coverage
coverage:
	export LOGLEVEL=WARN
	cargo install cargo-tarpaulin
	mkdir -p coverage
	cargo tarpaulin --exclude-files 'benches/**' --all-features --workspace --timeout 120 --out Xml

.PHONY: coverage-html
coverage-html:
	export LOGLEVEL=WARN
	cargo install cargo-tarpaulin
	mkdir -p coverage
	cargo tarpaulin --exclude-files 'benches/**' --verbose --all-features --workspace --timeout 120 --out Html --output-dir coverage

.PHONY: coverage-json
coverage-json:
	export LOGLEVEL=WARN
	cargo install cargo-tarpaulin
	mkdir -p coverage
	cargo tarpaulin --exclude-files 'benches/**' --verbose --all-features --workspace --timeout 120 --out Json --output-dir coverage

.PHONY: open-coverage
open-coverage:
	open coverage/tarpaulin-report.html

# Rule to show git log
git-log:
	@if [ "$(CURRENT_BRANCH)" = "HEAD" ]; then \
		echo "You are in a detached HEAD state. Please check out a branch."; \
		exit 1; \
	fi; \
	echo "Showing git log for branch $(CURRENT_BRANCH) against main:"; \
	git log main..$(CURRENT_BRANCH) --pretty=full

.PHONY: create-doc
create-doc:
	cargo doc --no-deps --document-private-items

.PHONY: check-spanish
check-spanish:
	cd scripts && python3 spanish.py ../src && cd ..

.PHONY: zip
zip:
	@echo "Creating $(ZIP_NAME) without any 'target' directories, 'Cargo.lock', and hidden files..."
	@find . -type f \
		! -path "*/target/*" \
		! -path "./.*" \
		! -name "Cargo.lock" \
		! -name ".*" \
		| zip -@ $(ZIP_NAME)
	@echo "$(ZIP_NAME) created successfully."


.PHONY: check-cargo-criterion
check-cargo-criterion:
	@command -v cargo-criterion > /dev/null || (echo "Installing cargo-criterion..."; cargo install cargo-criterion)

.PHONY: bench
bench: check-cargo-criterion
	cargo criterion --output-format=quiet

.PHONY: bench-show
bench-show:
	open target/criterion/reports/index.html

.PHONY: bench-save
bench-save: check-cargo-criterion
	cargo criterion --output-format quiet --history-id v0.4.6 --history-description "Version 0.3.2 baseline"

.PHONY: bench-compare
bench-compare: check-cargo-criterion
	cargo criterion --output-format verbose

.PHONY: bench-json
bench-json: check-cargo-criterion
	cargo criterion --message-format json

.PHONY: bench-clean
bench-clean:
	rm -rf target/criterion


.PHONY: workflow-coverage
workflow-coverage:
	DOCKER_HOST="$${DOCKER_HOST}" act push --job code_coverage_report \
       -P ubuntu-latest=catthehacker/ubuntu:latest \
       --privileged

.PHONY: workflow-build
workflow-build:
	DOCKER_HOST="$${DOCKER_HOST}" act push --job build \
       -P ubuntu-latest=catthehacker/ubuntu:latest

.PHONY: workflow-lint
workflow-lint:
	DOCKER_HOST="$${DOCKER_HOST}" act push --job lint

.PHONY: workflow-test
workflow-test:
	DOCKER_HOST="$${DOCKER_HOST}" act push --job run_tests

.PHONY: workflow
workflow: workflow-build workflow-lint workflow-test workflow-coverage

.PHONY: tree
tree:
	tree -I 'target|.idea|.run|.DS_Store|Cargo.lock|*.md|*.toml|*.zip|*.html|*.xml|*.json|*.txt|*.sh|*.yml|*.yaml|*.gitignore|*.gitattributes|*.gitmodules|*.git|*.gitkeep|*.gitlab-ci.yml' -a -L 3
