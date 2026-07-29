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

web-build:
    cd web && npm ci && npm run build
    git diff --exit-code -- web/dist

web-dev:
    cd web && npm run dev

test-api:
    cargo test -p kemuri-server

test-web:
    cd web && npm ci && npm run build

test-usage:
    bash packaging/tests/usage.sh

test-container:
    docker build -f packaging/container/Dockerfile -t kemuri:test .

test-load:
    cargo test -p kemuri-server scheduler

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

ci: fmt lint test test-web yaml release-check

ci-diff: fmt lint test

all: fmt lint test web-build
