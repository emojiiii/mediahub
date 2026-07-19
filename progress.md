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
