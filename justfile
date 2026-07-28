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
    cd web && npm install && npm run build

web-dev:
    cd web && npm run dev

ci: fmt lint test

all: fmt lint test web-build
