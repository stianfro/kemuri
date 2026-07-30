build:
    cargo build

build-release:
    cargo build --release

test:
    cargo test

lint:
    cargo clippy --all-targets --all-features -- -D warnings

fmt:
    cargo fmt --all -- --check

fmt-fix:
    cargo fmt --all

check:
    cargo check --all-targets --all-features

run *args:
    cargo run -- {{args}}

api-generate:
    mkdir -p openapi web/src/generated
    cargo run --quiet -p kemuri-server --example export_openapi > openapi/openapi.json
    cd web && npm ci && npm run generate-api

api-check: api-generate
    git diff --exit-code -- openapi/openapi.json web/src/generated/api.ts

web-build: api-generate
    cd web && npm run build
    git diff --exit-code -- web/dist

web-dev:
    cd web && npm run dev

docs-install:
    cd site && npm ci

docs-build: docs-install
    cd site && npm run build

docs-dev: docs-install
    cd site && npm run dev

docs-preview: docs-build
    cd site && npm run preview

docs-lint:
    python3 site/scripts/lint-docs.py

docs-test: docs-lint docs-build
    cd site && npm audit --audit-level=high

test-api:
    cargo test -p kemuri-server
    just api-check

test-web:
    cd web && npm ci && npm test && npm run build

test-browser:
    bash packaging/tests/browser.sh

test-usage:
    bash packaging/tests/usage.sh

test-container:
    bash packaging/tests/container.sh

test-load: build
    cargo test -p kemuri-server --test load
    python3 packaging/tests/load.py --checks 100 --duration 3 --interval 1s --concurrency 100 --api-readers 4 --output -
    python3 packaging/tests/load.py --checks 20 --duration 2 --interval 1s --probe https --failure-percent 50 --output -
    python3 packaging/tests/load.py --checks 20 --duration 2 --interval 1s --probe tls --failure-percent 50 --output -

load-test-local *args: build
    python3 packaging/tests/load.py {{args}}

bench:
    cargo bench --workspace --no-run

release-generate:
    dist generate

release-check:
    dist generate --check
    dist plan --output-format=json | jq -e '.releases | length == 1' >/dev/null

release-build *args:
    dist build {{args}}

yaml:
    find .github packaging -type f \( -name '*.yml' -o -name '*.yaml' \) -print0 | xargs -0 -r -n1 yq eval '.' >/dev/null

audit:
    cargo deny check
    cd web && npm audit --omit=dev --audit-level=high

ci: fmt lint test test-web docs-test yaml release-check

ci-diff: fmt lint test

all: fmt lint test web-build
