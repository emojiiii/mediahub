# Progress Log

## Pre-release crates remediation (2026-07-20)

- Re-read the planning skill, recovered the completed review context, and confirmed only review records were dirty before implementation.
- Added a six-phase remediation plan and dispatched core/app, image/local, and PostgreSQL/OpenAPI work in parallel.
- Agreed on a persistent upload-session storage-cleanup acknowledgement shared by app and PostgreSQL work.
- Began S3 storage-contract redesign and server integration review.

## Pre-release crates quality review (2026-07-20)

- Read the `planning-with-files` instructions and recovered the existing project review history.
- Confirmed the worktree was clean before starting this review.
- Added a review-only plan covering package-level, cross-crate, and tool-assisted validation.
- Inventoried all eight workspace packages and their source files/dependency metadata.
- Dispatched three read-only parallel reviews; the primary review is covering `mediahub-server` and cross-crate contracts.
- Ranked server modules by size and began scanning panic markers, module composition, and production entrypoints.
- Traced router middleware, HMAC replay protection, session/application authentication, CSRF helpers, and auth-rate-limit state.
- Built an initial HTTP handler matrix for authentication, permissions, and CSRF checks; no missing check has been accepted as a finding without following delegated helper calls.
- Scanned production code for ignored errors, unbounded buffering, task supervision, outbound URL handling, and SSRF defenses.
- Correlated server worker loops with application lease contracts and PostgreSQL lease fencing; identified a reproducible design mismatch between maximum batch size, sequential execution, and the fixed 30-second lease.
- Traced Application deletion through the server, repository method, schema foreign keys, contract tests, and UI-facing behavior; found incomplete aggregate cleanup and audit-retention drift.
- Audited transformed media loading against adapter-side image limits and found the limit is enforced after full-object allocation.
- Merged delegated core/app and adapter reviews; corroborated upload commit ambiguity, expired-session cleanup, S3 presigned replay/race, image intermediate-allocation, and metadata quota findings against current source.
- Received and recorded the PostgreSQL/OpenAPI review: tenant composite-FK gaps, mutable webhook cursors, idempotency fencing, and multiple generated-contract mismatches are now corroborated candidates/findings.
- Completed all four package-review ownership groups; started cross-crate validation and quality-tool execution.
- Format, workspace check, all-target/all-feature Clippy, and workspace test compilation passed.
- All locally runnable package/unit suites passed (81 unit tests plus the S3 wrapper contract); OpenAPI generated-file check passed.
- Destructive PostgreSQL and real-S3 contracts were not run because their isolated test environment variables/services are unavailable.
- Corroborated every release-blocking finding against current source and completed the severity-ordered review.
- No product source was modified; only `task_plan.md`, `findings.md`, and `progress.md` contain review records.

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

## README usage and deployment guide

- Rewrote the README opening guide with prebuilt-image deployment, source builds, Cloudflare Pages/pnpm Web deployment, configuration categories, and first-admin bootstrap steps.
- Documented Local and external S3 storage profiles, Resend email settings, JSON control-plane routes, native path API, WebDAV, bounded S3 gateway, health endpoints, and production TLS/backup boundaries.
- Corrected historical README/runbook wording that described supported S3 or Resend behavior as future/generic provider functionality.

## Container entrypoint and release tags

- Confirmed the root is intentionally a virtual workspace and `mediahub-server` owns the conventional binary entry point.
- Matched the production exit-0/no-log restart loop to the Docker dependency placeholder binary being retained after the real-source overlay.
- Added deterministic cleanup of all workspace Release artifacts after copying real sources, preserving third-party dependency caches while preventing placeholder packaging.
- Restricted default-branch image metadata to `master` and `latest`; removed version-tag workflow triggers and SHA/semver/PR image tags.
- Actionlint passed after removing the obsolete tag trigger and metadata entries.
- Built `mediahub:entrypoint-fix`; the source layer removed placeholder workspace artifacts and recompiled the real server binary.
- Negative runtime check printed the required-key configuration error and exited 1 instead of silently exiting 0.
- Full temporary PostgreSQL smoke passed with real startup logs, restart count 0, non-root runtime, and healthy live/readiness endpoints; task-owned containers/network were removed.
- Final actionlint, Cargo check, diff check, image metadata inspection, and tag scan passed; only `master` and `latest` remain publishable from the default branch.

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

## Pre-release crates remediation

- Resumed the implementation after the cross-crate repair pass and recovered the current plan, findings, worktree, and isolated PostgreSQL environment.
- Reused the three completed review agents for non-overlapping OpenAPI/server, S3/upload-session, and core/local regression coverage.
- Confirmed ordinary stale-upload reconciliation is wired into the lifecycle worker and added a PostgreSQL contract for cutoff handling and multipart completion fencing.
- Added S3 and application tests for signed overwrite protection, delayed terminal cleanup, temporary-object removal, and final-object retention.
- A final storage review found that Local completed direct-upload cleanup could delete the active final object; the Local owner is fixing it with a focused regression test.
- Local completed cleanup is now guarded and its regression test confirms the final object and MIME sidecar survive terminal cleanup.
- The destructive PostgreSQL repository contract passed against the isolated database, including the new stale ordinary-upload cutoff and multipart fencing assertions.
- Final read-only reviews found and closed four additional release risks: slow-upload reconciliation races, high-bit-depth image allocation undercounting, unbounded webhook DNS/attempt time, and incomplete AsyncJob date-time/nullability schemas.
- Added migration `0010_ordinary_upload_fencing.sql`: ordinary uploads now persist the actual temporary key, lease token, and expiry; upload and reconciliation owners heartbeat; PostgreSQL claims use `FOR UPDATE SKIP LOCKED`; stale tokens cannot renew, commit, or abort.
- Reconciliation now requires final size, MIME, and a trustworthy SHA-256. S3 stores digest metadata and falls back to an ETag/version-fenced streaming hash when metadata is absent.
- S3 and Local promotion cleanup failures remain in the durable `uploading` recovery protocol; the final object survives and temporary deletion is retried before activation.
- Image intermediate limits now account for actual sample width/bands, including RGBA16 cover operations.
- Webhook DNS is bounded to 5 seconds and the whole attempt to 25 seconds, strictly below its 30-second lease.
- Webhook pagination cursors now contain only immutable `history_id`; legacy tokens containing `updated_at` remain accepted.
- OpenAPI AsyncJob/Item time fields, required nullable fields, enums, bounds, and sensitive-field exclusions now match the runtime response DTOs.
- Final validation passed: formatting, workspace check, all-target/all-feature Clippy with warnings denied, test compilation, OpenAPI generation/check, `git diff --check`, and the complete workspace test suite.
- Final test counts: Image 11, Local 10, PostgreSQL unit 6 plus destructive contract 1, S3 unit 6 plus wrapper contract 1, App 21, Core 35, OpenAPI 10, Server lib 8, Server binary 75. The real S3 contract remains the single expected ignored test because `MEDIAHUB_TEST_S3_*` is not configured.
- Removed the isolated `mediahub-codex-test` PostgreSQL container, network, and test volume after all destructive/server database tests passed.
- Live creation of 空白测试应用 navigated to a dashboard with 0 objects, 0 Buckets, and 0 B usage.
# Web Wrangler Deployment Refactor Progress

- Started audit of the Web build and Cloudflare deployment contract.
- Confirmed the reference Wrangler configuration uses Worker static assets, while this repository is a Vite SPA producing `dist`.
- Confirmed the current Vite configuration has separable production and test responsibilities.
- Confirmed SPA route fallback is a deployment requirement and the existing README deployment instructions need updating.
- Baseline full build stopped at the pre-existing stale OpenAPI client check; continuing with isolated TypeScript/Vite/viewer-contract checks.
- Added the Vite/Vitest split, Wrangler assets configuration/scripts, documentation, and obsolete-file removals; dependency installation now needs an explicit pnpm build-script policy.
- Dependency installation passes; all 26 Vitest files/128 tests and both Wrangler dry-runs pass. Fixing one hoisted config literal type and the explicit top-level environment selector warning.
- TypeScript now passes, Vite production output rebuilds successfully, the viewer chunk/asset verifier passes, and both final Wrangler dry-runs pass without environment-selection warnings.
- Final repository checks pass; investigating a timed-out local Wrangler live-server smoke-test command before handoff.
- Wrangler logs identified a UTC compatibility-date boundary issue; pinned the date to 2026-07-20 before retrying the live server.
- Live Wrangler serves the deep SPA route successfully on port 8787. Regenerated the previously stale TypeScript API client so the complete deployment build can now be tested.
- The regenerated contract exposed one required Webhook update field; implementing an explicit non-rotating update before the final full build.
- Full production build, 27 test files/129 tests, frozen install, diff checks, and both final Wrangler dry-runs pass. Restarting the local dev server after Vite replaced its watched output directory.
- Restarted Wrangler after the final build; the latest SPA is available at http://127.0.0.1:8787 and deep routes resolve correctly.
- Completed the Web Wrangler deployment and Vite/Vitest configuration refactor.
