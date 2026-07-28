# Contributing

1. Fork the repository.
2. Create a feature branch.
3. Make your changes. Ensure `cargo fmt`, `cargo clippy -- -D warnings`, and `cargo test` pass.
4. Submit a pull request.

## Development

```sh
cargo build
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

The web UI is in `web/`. Build with `cd web && npm install && npm run build`.

## Code Style

- No unsafe code (denied by lint).
- No comments in code.
- Conventional commits for git messages.
