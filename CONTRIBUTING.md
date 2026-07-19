# Contributing

MediaHub is a Rust workspace with a Vite/React console. Keep changes focused,
preserve the OpenAPI contract, and add regression coverage for behavior changes.

## Local checks

Run the same checks used by CI before opening a pull request:

```powershell
cargo fmt --check
docker compose up -d postgres
$env:MEDIAHUB_TEST_POSTGRES_URL = 'postgres://mediahub:mediahub-local-only@127.0.0.1:5432/mediahub_contract'
$env:DATABASE_URL = $env:MEDIAHUB_TEST_POSTGRES_URL
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
Set-Location web
npm ci
npm test
npm run build
```

The PostgreSQL contract test is destructive. Use a dedicated test database and
never point `MEDIAHUB_TEST_POSTGRES_URL` at development or production data.

Open a normal GitHub issue for bugs and feature proposals. Report security
issues privately according to `SECURITY.md`.
