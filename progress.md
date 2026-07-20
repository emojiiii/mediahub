# Progress Log

## Real backend wiring

- Acknowledged that the prior validation covered Mock behavior only and did not validate the completed backend integration.
- Started auditing runtime configuration so real API usage becomes the default and Mock becomes explicit.
- Scope was tightened by the user: remove Mock entirely and make direct backend access the only runtime path.
- Deleted `web/src/api/mock.ts` and created a real-backend-only `web/src/api/index.ts` facade.
- Removed demo login defaults and all `app_studio`/`app_marketing` runtime route fallbacks.
- Replaced the seed-dependent App integration suite with backend-independent component/helper tests.
- Gated Playwright workflows on explicit real-backend credentials and Application ID instead of demo seeds.
- Added direct-backend CORS/cookie pass-through to Compose and documented the real-only console runtime.
- Runtime Mock residue scan passed.
- First full frontend test run exposed three test-only cleanup failures; focused fixes are in progress.
- Diagnosed the focused-suite transform failure as patch-output truncation in the extracted real API facade.
- Recovered the affected real-adapter span and removed the stale ApplicationSwitcher assertion.
- Focused frontend tests: 16 passed across 3 files.
- TypeScript build check passed.
- Full frontend test suite: 26 files, 128 tests passed.
- Production build and OpenAPI/generated-client checks passed; `web/dist` now contains the real-backend-only bundle.
- Docker login page loaded with empty credential fields; submitted the previously reported credentials and began inspecting the resulting backend/browser state.
- Live Docker authentication with `admin@example.com` / `admin` succeeded against the real backend and loaded backend-owned dashboard data.
- Built-bundle residue scan and credentialed CORS preflight both passed.
- Compose configuration validation passed.
- Playwright real-backend E2E specification discovery passed (2 workflows).
- Credential-gated E2E run completed with 2 expected skips because dedicated E2E credentials were not supplied.
- Final real API facade/file existence checks passed; all current task phases are complete.

## 2026-07-19

- Read the planning-with-files instructions.
- Session catch-up was blocked by a Windows sandbox ACL error.
- Confirmed the production entrypoint and measured the original file at 7,432 lines.
- Extracted responsibility-oriented implementation files.
- Reduced main.rs to 391 lines.
- Made bootstrap.rs and workers.rs real child modules with a narrow pub(super) interface.
- Ran cargo fmt --all.
- Ran cargo check -p mediahub-server: passed.
- Ran cargo test -p mediahub-server --no-run: passed.
- Ran focused tests media_directory_cursor_round_trips_and_isolated_from_flat_cursor and webhook_events_must_be_known_and_are_normalized: passed.
- A parallel focused-test permission review timed out; a single-test retry passed. Remaining test risk: the full suite includes database-backed tests and was not run without the required external database configuration.
## Workspace-wide second pass

- Split server S3 HTTP, WebDAV, handler, and DTO files into responsibility-oriented implementation files.
- Moved local and S3 adapter unit tests into dedicated tests.rs files.
- Ran cargo fmt --all.
- Ran cargo check --workspace: passed.
- Ran cargo test --workspace --no-run: passed.
- Final sequential cargo fmt check and cargo check --workspace: passed.
- Full runtime test suites still require external database/storage configuration.
- Local adapter library tests: 6 passed.
- S3 adapter library tests: 3 passed.
- Split PostgreSQL media.rs and s3_multipart.rs into focused repository implementation files.
- Re-ran cargo test --workspace --no-run after the PostgreSQL split: passed.
- Final cargo fmt --all -- --check: passed.
- Final cargo check --workspace: passed.
## Final workspace pass

- Created dedicated Docker database mediahub_codex_contract and ran repository_contract: 1 passed.
- Core library tests: 25 passed.
- App library tests: 16 passed.
- Image adapter library tests: 9 passed.
- OpenAPI library tests: 7 passed.
- Final cargo fmt --all -- --check: passed.
- Final cargo check --workspace: passed.
- Full cargo test --workspace was not run because some suites are destructive/integration-heavy; the relevant contract and library suites were run individually.
## Open-source release and container audit

- Started 2026-07-19.
- Confirmed the worktree was clean at `97ed657` before beginning this audit.
- Recovered and retained the completed historical planning context.
- Began repository, GitHub Actions, Docker, and public-release inventory.
- Found no container build/publish workflow; the checked-in CI currently validates source only.
- Identified a missing root MIT license file despite the workspace manifest declaring MIT.
- Confirmed the current Docker image packages the API/worker only, while the web console remains a separate Pages/Vite deployment.
- Completed the repository/workflow/container/documentation inventory.
- Found stale public README status/configuration text; the current runbook is more accurate but not a concise image-based deployment guide.
- Initial tracked-file and one-commit history scan found no likely real credentials or private keys.
- Reproduced the Web CI dependency-install blocker: npm reports `ENOLOCK` because the workflow-required `web/package-lock.json` does not exist.
- Generated npm lockfile v3 and successfully queried the official npm advisory service.
- Recorded 5 npm audit findings; Vite is fixable, while the SheetJS/xlsx chain requires a deliberate replacement or mitigation.
- Gitleaks reported one redacted finding pending path/rule triage; the first RustSec container approach was invalid and will be replaced.
- Triaged the Gitleaks hit as a fixed runbook example key, not a credential; planned secure random-key generation in the documentation.
- Verified a patched official SheetJS 0.20.3 tarball and a compatible Vite 8/Vitest 4 upgrade set.
- First canonical lockfile regeneration failed because npm's host-replacement option also rewrote the intentional SheetJS CDN URL; identified the 404 in the npm debug log and removed that option for the retry.
- Rebuilt the npm lockfile using canonical sources; clean `npm ci` passes and npm audit reports zero vulnerabilities.
- Actionlint passed. Hadolint found the checksum-pipeline shell issue plus intentionally unpinned apt-package warnings.
- RustSec found two reachable quick-xml denial-of-service advisories and one non-enabled rsa lockfile advisory.
- Reproduced the container's fresh-volume permission failure as UID 10001 against root-owned `/data`.
- Implemented the npm/Rust dependency fixes, non-root volume ownership, health check, OCI labels, GHCR multi-architecture publishing, secure Compose defaults, and public-project policy/docs.
- Post-change quick validation passed: Actionlint, Compose interpolation, Hadolint with only DL3008 excluded, Gitleaks with zero findings, and `git diff --check`.
- RustSec now reports only the disabled sqlx-mysql/rsa lockfile path; no MediaHub target enables that dependency.
- Frontend verification after the Vite/Vitest upgrade: 26 files and 128 tests passed.
- First Rust workspace run found one stale source-structure test after the earlier PostgreSQL file split; fixed the test to scan the actual included implementation files.
- Clippy exposed and fixed one current-stable lint; Vite 8 exposed a lazy-chunk contract regression, so the frontend target is being adjusted to Vite 7.3.6/plugin-react 5.2.0 before rerunning build.
- Clippy now passes. The first incremental Vite 7 lock update omitted an optional WASI dependency, so npm correctly rejected it as non-reproducible; a clean lock resolution is in progress.
- User clarified the final ownership boundary: Cloudflare builds the pnpm Web UI; GitHub Actions and the deployment image cover backend crates only.
- Removed the GitHub Web job and restricted backend CI/image triggers to crates, Cargo files, Docker inputs, and their own workflow files.
- Replaced package-lock with a pnpm 10.23.0 lockfile and excluded `web/` from the Docker build context.
- Built `mediahub:open-source-audit`; verified non-root storage writes, shared libraries, OCI metadata, live/readiness HTTP 200, and Docker healthy state.
- Removed the temporary API/PostgreSQL audit containers and retained the local image.

## Latest dependency upgrade

- Started from clean commit `8c834ed` on 2026-07-19.
- Began direct dependency inventory for pnpm Web, Cargo workspace, Actions, and Docker build images.
- Captured the Web major-upgrade set and Cargo's compatible lockfile updates; `cargo-outdated` is being installed for the full direct-major matrix.
- Installed cargo-outdated 0.19.0 and captured direct Cargo plus official Actions latest-version matrices.
- Began pnpm 11 migration; moved the xlsx override to the pnpm 11 workspace configuration.
- First pnpm 11 latest update was stopped by release-age/integrity policy before dependency compatibility tests; policy configuration is being inspected.
- Exact pnpm policy exceptions passed the first checks, but the xlsx transitive URL override hit `blockExoticSubdeps`; investigating scoped configuration.
- Completed the pnpm 11 latest resolution after adding the required SheetJS policy exception; `pnpm outdated` reports no remaining Web updates.
- Ran the upgraded Web test suite: 26 files and 128 tests passed.
- Confirmed TypeScript 7.0.2 breaks `openapi-typescript` 7.13.0 at runtime; selected TypeScript 5.9.3 as the latest compatible release.
- Removed obsolete React Router 6 future flags after the React Router 7 type check rejected them.
- Added a Vite 8/Rolldown code-splitting group for `docx-preview` to restore the viewer lazy-loading contract.
- Updated the viewer-chunk verifier to distinguish Vite 8 lazy preload metadata from actual static imports while retaining the initial HTML and import-chain checks.
- Migrated PDF.js static-copy targets to the v4 flattening option so the existing public asset URLs are preserved.
- Production Web build now passes on Vite 8.1.5 with all lazy-viewer and local PDF asset checks.
- Official npm audit exposed a stale Mammoth CLI dependency on vulnerable lodash 3; added a pnpm override to latest argparse 3.0.0 to remove that chain.
- Final upgraded Web verification passed: no peer issues, 26 test files/128 tests, production build/lazy asset contract, and zero official npm audit findings.
- Updated Cargo's compatible lock set and raised workspace/direct manifest floors to the latest same-major releases, including the previously exact-pinned AWS crates.
- Began the crypto-major migration with aes-gcm 0.11, hmac 0.13, sha2 0.11, and md-5 0.11; password-hash remains on the latest line compatible with stable argon2.
- Migrated SHA-256 string formatting to explicit hex encoding for digest 0.11.
- Migrated aes-gcm 0.11 to generated typed nonces/keys and checked nonce conversion; imported hmac 0.13's explicit `KeyInit` trait.
- Updated rand to 0.10.2; the project's `rand::random` API remains compatible.
- Updated reqwest to 0.13.4 and migrated its renamed rustls feature, allowing Cargo to remove the direct reqwest 0.12 copy.
- Updated tower-http to 0.7.0 for the existing CORS, request-id, and trace layers.
- Began the SQLx 0.9.0 database API migration.
- Updated the shared PostgreSQL query helper to accept static SQL text as required by SQLx 0.9.
- Updated the Docker builder to Rust 1.97.0 and all GitHub Actions to their latest verified formal tags; the workflow remains backend-only.
- Actionlint and Hadolint passed for the upgraded backend-only workflow and Dockerfile.
- Full PostgreSQL-backed workspace tests passed; the external real-S3 test remained the single expected ignored test.
- CI feature checks and Clippy with all targets/features passed.
- RustSec scanned 360 lockfile dependencies against 1,166 official advisories with zero vulnerabilities.
- Built `mediahub:dependency-upgrade` successfully and verified its non-root writable runtime, health endpoint, OCI metadata, and libvips linkage.

## Libvips CI compatibility

- Investigated the reported GitHub failure and confirmed it is caused by generated libvips 8.18 bindings running against an older system libvips.
- Selected a minimal WebP suffix-option encoding path for cross-version compatibility.
- First 8.14.1 container attempt stopped before tests because a login shell removed Cargo from PATH; the second stopped during an external Debian mirror 502.
- The first completed 8.14.1 test run exposed `keep` as the same generated-binding compatibility class for JPEG; migrated all three savers to explicit suffix options.
- Debian bookworm libvips 8.14.1 image suite passed: 11 tests, 0 failures, including both reported transcode failures.
- Pinned libvips 8.18.4 image suite passed: 11 tests, 0 failures; the release `mediahub-server` build also passed.
- Local format check, diff check, focused adapter tests, `docker-libvips` server check, and workspace Clippy with all targets/features passed.
- Rebuilt `mediahub:dependency-upgrade`; runtime smoke passed as non-root user with Docker health `healthy` and HTTP 200 live/readiness responses.

## Resend email integration

- Started direct Resend integration work from a clean worktree.
- Planned API-contract verification, backend implementation, configuration/documentation migration, and focused regression coverage.
- Replaced the generic token-template webhook client with a dedicated Resend client and typed verification/reset email templates.
- Added public Web-origin validation, URL-encoded action links, a 10-second request timeout, hashed idempotency keys, and validated Resend success responses.
- Focused Resend/template/config tests passed: 5 tests, 0 failures.
- Migrated `.env.example`, Compose, README, and the runbook from the generic provider URL/token contract to Resend API key, verified sender, and public Web origin configuration.
- Full `mediahub-server` verification passed against isolated PostgreSQL: 8 library tests and 72 binary tests.
- Workspace Clippy with all targets/features, format check, diff check, and Compose configuration validation passed.
- Built `mediahub:resend`; runtime smoke passed as non-root user with Docker health `healthy` and HTTP 200 live/readiness responses. No real Resend request was sent.

## Application resource isolation

- Confirmed backend requests and React Query keys are already Application-scoped.
- Identified module-global Mock resource arrays as the source of cross-Application leakage.
- Chose per-app resource stores with empty state for marketing and newly created Applications.
- Defined the create-application dialog visual revision.
- Added per-Application Mock resource stores with distinct studio and marketing seed data.
- New Applications now receive empty Buckets, objects, access keys, Webhooks, deliveries, jobs, and upload sessions.
- Updated Dashboard and Admin aggregation to read the appropriate stores.
- TypeScript project build passed after the data-layer change.
- Added focused tests for Application resource isolation and the create dialog; both passed.
- Browser skill runtime was blocked by the Windows ACL helper; Playwright CLI prerequisites are present, so visual verification will use the CLI fallback.
- Production build passed and the live Playwright session loaded the updated Mock console at app_studio.
- Live dashboard confirmed app_studio shows its own images/videos resources before switch testing.
- Desktop Playwright snapshot confirmed the application switcher exposes distinct studio and marketing entries after the build.
- Live create dialog exposes the compact structure, isolated-resource copy, full-width name field, and disabled submit state before input.
- Live dialog cancellation returned cleanly to the studio dashboard.
- Reopened the live switcher at desktop width; marketing is available as a separate selectable Application.
- Live switch to app_marketing showed 1 object, 1 campaign-assets Bucket, and 4.1 MB instead of studio values.
- Live marketing Buckets page contains only campaign-assets (1 object, 4.1 MB), confirming it no longer reuses studio Buckets.
- Live marketing Objects page selected campaign-assets and exposed only its campaigns directory.
- Live marketing Access Keys page contains campaign-publisher/mh_ak_marketing rather than studio production-uploader.
- Live marketing Webhook page contains only https://marketing.example.com/hooks/assets; marketing key and Webhook isolation are confirmed.
- Began live new-Application creation from app_marketing to verify the empty initial state.
- Live create dialog reopened from marketing with empty input and disabled submit, confirming cancellation reset.
- Live create dialog enabled submit only after a non-empty Application name was entered.
- Live creation of 空白测试应用 navigated to a dashboard with 0 objects, 0 Buckets, and 0 B usage.
