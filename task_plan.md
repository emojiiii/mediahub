# Code Structure Refactor Plan

## Goal

Improve structure across the Rust workspace without changing behavior. Reduce oversized modules, separate protocol/adapter responsibilities, and keep tests close to their owning components.

## Phases

- [completed] 1. Inventory repository, workspace entrypoints, and main.rs
- [completed] 2. Extract responsibility-oriented source files
- [completed] 3. Tighten the startup/worker module boundary and preserve test access
- [completed] 4. Run formatting, compilation, and focused tests
- [completed] 5. Final review and handoff
- [completed] 6. Extend modularization to server protocols, handlers, DTOs, and storage adapters
- [completed] 7. Split PostgreSQL media and multipart repository implementations
- [completed] 8. Validate PostgreSQL contracts against a dedicated Docker database
- [completed] 9. Refactor remaining large app, OpenAPI, image, core, and control-plane modules
- [completed] 10. Run full formatting, workspace compilation, and focused runtime tests

## Errors Encountered

| Error | Attempt | Resolution |
| --- | --- | --- |
| README replacement patch omitted `+` prefixes on code-block lines | 1 | No partial write occurred; split the documentation edit into smaller validated hunks |
| `cargo outdated` is not installed | 1 | Install the current locked `cargo-outdated` CLI, then inventory direct major upgrades |
| Action tag parser rejected one-segment tags such as `v1` as `System.Version` | 1 | Ignore one-segment aliases; use the highest complete semantic tag returned for each official action repository |
| pnpm 11 rejected the pnpm 10 project declaration and ignored package.json overrides | 1 | Update `packageManager` to 11.15.0 and move the xlsx override to `pnpm-workspace.yaml` before resolving dependencies |
| pnpm 11 supply-chain policy rejected newly published packages and the external SheetJS tarball | 1 | Inspect pnpm 11 policy settings and add the narrowest explicit exceptions required by the user-requested latest versions and verified SheetJS source |
| pnpm 11 blocked the SheetJS URL override as an exotic transitive dependency | 1 | Check for a package-scoped `blockExoticSubdeps` exclusion before considering an explicit project-level opt-out |
| Session catch-up script rejected by Windows sandbox ACL | 1 | Recovered context from current workspace and git status |
| First sibling-module split produced private-item errors | 1 | Reworked handlers into crate-level included implementation files; kept only bootstrap/workers as true modules |
| Mechanical extraction left orphaned attributes and duplicate entrypoint declarations | 1 | Corrected file boundaries and rebuilt main.rs tail |
| Test build missed a worker helper import | 1 | Added explicit root import for the existing test contract |
| One cargo-check permission review timed out | 1 | Retried once and completed successfully |
| Parallel focused-test permission review timed out | 1 | Retried a single focused test and it passed |
| Parallel final-check permission review timed out | 1 | Retried format and workspace checks sequentially; both passed |
| PostgreSQL media impl was initially split inside an impl block | 1 | Closed and reopened complete impl PostgresRepository blocks at file boundaries |
| Multipart parent retained an orphan async_trait attribute | 1 | Removed the duplicate parent attribute; kept it with the trait impl |
| Async-job recovery script wrote PowerShell error output into async_job_error.rs | 1 | Reconstructed the public error/result API and validation functions from all call sites, then ran core tests |
## Current Task: Application Resource Isolation

### Goal

Improve the create-application dialog and ensure each Application owns an isolated set of Buckets, objects, access keys, and Webhooks in the console data layer.

### Phases

**Status:** complete

- [completed] 1. Trace Application selection and resource ownership across Mock and backend APIs
- [completed] 2. Design and implement Application-scoped Mock resource stores
- [completed] 3. Refine the create-application dialog layout and states
- [completed] 4. Add regression tests for cross-Application isolation and dialog behavior
- [completed] 5. Run frontend tests/build and manually verify the switching workflow

### Current Task Errors

| Error | Attempt | Resolution |
| --- | --- | --- |
| A parallel source read failed because one exploratory `rg` pattern had no match | 1 | Switched to already located line ranges and `Promise.allSettled`; no code state was changed |
| Final-verification findings patch used a heading that exists only in `task_plan.md`, not `findings.md` | 1 | No file was changed; re-read both tails and appended findings under a stable end-of-file anchor |
| A combined code/plan/progress patch used an unstable historical mojibake line as context | 1 | The patch was atomic and changed nothing; split product and record edits, using ASCII anchors |
| A final test lookup used `rg` without tolerating a legitimate no-match exit code | 1 | No file was changed; switched the lookup to `Select-String` and added a direct cursor compatibility test |
| The first S3 cleanup regression patch assumed a stale end-of-test assertion | 1 | The patch was atomic and changed nothing; re-read the exact function and split the edit into stable hunks |
| A cursor test command used `--exact` without the `tests::` module prefix and ran zero tests | 1 | Re-ran with the complete test name and verified one actual passing test |
| Final-verification findings patch used a heading that exists only in `task_plan.md`, not `findings.md` | 1 | No file was changed; re-read both tails and append findings under a stable end-of-file anchor |
| Tried to invoke `spawn_agent` through `functions.exec`, where collaboration tools are unavailable | 1 | Spawn/follow up agents directly through the collaboration namespace |
| New agent slots were occupied by completed review agents | 1 | Reused the completed review agents with `followup_task` for image/local and PostgreSQL/OpenAPI fixes |
| Accidentally called the exec-cell wait tool without a cell ID while checking agent state | 1 | Used `collaboration.list_agents` and direct follow-up tools instead |
| apply_patch could not read workspace files because the Windows sandbox ACL helper failed | 1 | Use guarded .NET text replacement for this session |
| Exact replacement depended on platform newline bytes and did not match applyMockBatch | 1 | Switch to function-boundary and signature-only replacements |
| In-app browser runtime exited because the Windows sandbox ACL helper failed | 1 | Fall back to the repository Playwright CLI workflow for local visual verification |
| Full App test run timed out waiting for the existing lazy video viewer test | 1 | Run the video test alone, then rerun the full file after build/cache stabilization |
| playwright-cli --help printed successfully but hit a Windows libuv handle assertion on exit | 1 | Avoid the help path and open a named browser session directly |
| Playwright first click targeted the off-canvas desktop switcher at the default mobile viewport | 1 | Resize to 1440x900 and refresh element references before interacting |
| view_image could not read the Playwright screenshot because of the workspace ACL helper | 1 | Copy the screenshot to the allowed visualization root before inspection |
| Playwright goto reloaded the page and reset the in-memory Mock login session | 1 | Use only SPA menu/link clicks while verifying Mock resources |

## Current Task: Real Backend Wiring

### Goal

Remove the runtime Mock implementation and all demo data/accounts so the console always talks directly to the completed MediaHub backend in local and Docker deployments.

### Phases

**Status:** complete

- [completed] 1. Inventory every runtime and test dependency on the Mock API
- [completed] 2. Extract the real API facade and delete Mock data/mode selection
- [completed] 3. Rewrite or remove Mock-dependent tests and documentation
- [completed] 4. Run frontend tests/build and backend configuration checks
- [completed] 5. Verify direct backend login and Application-scoped resources

### Current Task Errors

| Error | Attempt | Resolution |
| --- | --- | --- |
| Initial multi-file planning patch did not match a mojibake progress-log line | 1 | Patch only stable ASCII anchors and keep implementation changes scoped |
| First post-removal frontend test run reported three failures, including one stale ApplicationSwitcher fixture label | 1 | Inspect focused failures, update assertions/types, and rerun |
| Generated real-facade patch was truncated around a very long source line and wrote an ellipsis marker into `api/index.ts` | 1 | Recover the affected backend adapter span from local build/cache artifacts, then use small apply_patch hunks only |
| Browser wait for the generic login error title timed out after submitting the reported credentials | 1 | Inspect a fresh URL/DOM snapshot and browser logs before choosing a more specific assertion |
| Completion helper initially reported 0/2 because the recovered plan used non-template `[completed]` markers | 1 | Added explicit template-compatible complete status to both task phase sections |

## Current Task: Open-source Release and Container Audit

### Goal

Make the GitHub Actions and container build reproducible for deployment, fix confirmed release blockers, and produce an evidence-backed pre-open-source review.

### Phases

**Status:** complete

- [completed] 1. Inventory repository metadata, GitHub Actions, Docker/Compose, and documented deployment contract
- [completed] 2. Audit secrets, licenses, generated artifacts, dependencies, and release-facing configuration
- [completed] 3. Fix confirmed CI, container, dependency, and open-source readiness issues
- [completed] 4. Run local backend CI-equivalent checks and build/test the deployment image
- [completed] 5. Inspect the image and deliver the remaining-risk checklist

### Current Task Errors

| Error | Attempt | Resolution |
| --- | --- | --- |
| Initial planning update could not match a historical mojibake line in `progress.md` | 1 | Retry with stable ASCII section-heading anchors and preserve all historical content |
| `npm audit` failed with `ENOLOCK` because `web/package-lock.json` is absent | 1 | Treat as a confirmed CI blocker; generate a lockfile, then validate with a clean `npm ci` and audit |
| npm mirror returned 404 for the security advisory endpoint | 1 | Re-ran the audit against `https://registry.npmjs.org` and obtained a valid report |
| `rustsec/rustsec:latest` container image does not exist | 1 | Use the supported `cargo-audit` CLI instead of retrying the invalid image |
| Initial `cargo audit` advisory-database fetch timed out and left no usable cache | 1 | Download the official advisory database archive separately, then run `cargo audit --no-fetch --db ...` |
| npm lock regeneration rewrote the SheetJS CDN tarball to a nonexistent npm-registry path and timed out | 1 | Remove `replace-registry-host=always`; use the official registry for registry packages while preserving the explicit SheetJS CDN URL |
| Hadolint invocation used unsupported PowerShell stdin redirection | 1 | Re-ran Hadolint by bind-mounting the Dockerfile read-only and obtained the actual warnings |
| Workspace tests failed because a PostgreSQL source-contract test scanned only refactored facade files | 1 | Concatenate the facade and its included implementation files so the existing SQL invariants are checked where they now live |
| Clippy on current stable rejected a nested `if let` with `collapsible_if` | 1 | Collapse the condition without changing validation behavior |
| Vite 8 build passed compilation but failed the repository's lazy-viewer chunk contract because Rolldown merged the DOCX asset | 1 | Use the compatible patched Vite 7 line and React plugin 5.2, then regenerate the lockfile and rerun the build |
| Incremental npm lock update omitted `@emnapi/wasi-threads`, so `npm ci` rejected package/lock drift | 1 | Remove the generated lock and ignored `node_modules`, then resolve from an empty dependency tree and verify with `npm ci` |
| First runtime image check used a database populated by tests with an empty anonymous storage volume | 1 | Confirmed the consistency guard was correct; created a fresh isolated database and reran the image successfully |
| Empty PostgreSQL existence query returned PowerShell `$null` and `.Trim()` failed | 1 | Use null-safe string matching before creating the isolated image-audit database |

## Current Task: Latest Dependency Upgrade

### Goal

Upgrade direct Web, Rust, GitHub Actions, and build-image dependencies to their latest viable releases, migrate breaking APIs, and prove compatibility with full tests and production builds.

### Phases

**Status:** complete

- [completed] 1. Inventory every outdated direct dependency and classify breaking upgrades
- [completed] 2. Upgrade the pnpm Web workspace and restore all UI/build contracts
- [completed] 3. Upgrade Rust, GitHub Actions, and Docker build dependencies
- [completed] 4. Run full Web and backend verification, security audits, and image checks
- [completed] 5. Document any dependency that cannot safely move to latest

### Current Task Errors

| Error | Attempt | Resolution |
| --- | --- | --- |
| PowerShell parsed a quoted `rg` pattern incorrectly during a read-only source scan | 1 | Re-ran the scan with a single-quoted literal pattern |
| `openapi-typescript` crashed under TypeScript 7 while generating the client | 1 | Confirmed the latest 7.13.0 release requires TypeScript `^5.x`; retain latest compatible TypeScript 5.9.3 |
| React Router 7 rejected the obsolete `BrowserRouter.future` migration flags | 1 | Remove the flags because their v7 behavior is now the default |
| Vite 8/Rolldown merged `docx-preview` into the lazy object-viewer chunk | 1 | Add an explicit Rolldown code-splitting group that preserves the existing DOCX lazy-load contract |
| `vite-plugin-static-copy` 4 preserved the full source path for PDF.js support files | 1 | Enable `rename.stripBase` for CMaps and standard fonts so their public URLs remain stable |
| The configured npm mirror does not implement the audit API | 1 | Run pnpm audit explicitly against the official npm registry |
| Latest open-file-viewer pulled a vulnerable lodash 3 through an unused Mammoth CLI dependency | 1 | Override the unused argparse 1 dependency to latest 3.0.0, which removes lodash, then re-run tests/build/audit |
| digest 0.11 removed `LowerHex` from digest output arrays | 1 | Encode the seven SHA-256 outputs explicitly with the existing `hex` dependency |
| The local adapter did not previously depend on `hex` | 1 | Add the workspace `hex` dependency used by the digest 0.11 migration |
| An explicit rand 0.9 lockfile update target no longer existed after resolution | 1 | Confirmed Cargo had already converged the direct dependency on rand 0.10.2; workspace check passed |
| SQLx 0.9 requires the reusable query helper's SQL text to outlive its async query | 1 | Tighten the helper parameter to `&'static str`; its only caller passes a SQL literal |
| Docker Hub manifest lookup timed out before returning Rust image metadata | 1 | Keep the verified stable Rust 1.97.0 target and validate the tag through the actual deployment-image build |
| Online cargo-audit database refresh timed out and left an incomplete cache | 1 | Download the official RustSec database archive and audit the lockfile with `--no-fetch --db` |
| First runtime smoke test skipped database creation due PowerShell empty-output truthiness | 1 | Explicitly create and verify the isolated runtime database before restarting the image |
| The first `ldd` filter lost shell quoting after Docker argument forwarding | 1 | Run `ldd` directly as the container entrypoint and filter its output in PowerShell |
| Hadolint's cached image had no entrypoint | 1 | Invoke its declared `/bin/hadolint` command explicitly |

## Current Task: Libvips CI Compatibility

### Goal

Make the libvips image tests and production encoder work with both the GitHub runner's distro libvips and the Docker image's pinned libvips 8.18.4.

### Phases

**Status:** complete

- [completed] 1. Reproduce the CI error from the reported output and trace the generated binding behavior
- [completed] 2. Replace all generated saver-option calls with minimal cross-version option paths
- [completed] 3. Run image tests against an older distro libvips and pinned libvips 8.18.4
- [completed] 4. Re-run formatting, Clippy, workflow-equivalent checks, and deployment-image smoke tests

### Current Task Errors

| Error | Attempt | Resolution |
| --- | --- | --- |
| GitHub libvips tests reject `webpsave_buffer` option `exact` | 1 | The 8.18-generated Rust binding passes every option field; switch WebP encoding to a suffix option string containing only long-supported `Q` and `strip` |
| First old-libvips container used a login shell that removed Cargo from PATH | 1 | Re-run with a non-login `bash -c` shell |
| Debian package mirror returned HTTP 502 for `libsz2` | 1 | Add APT acquisition retries and `--fix-missing` before running the 8.14.1 test suite |
| Libvips 8.14.1 also rejects the generated JPEG option `keep` | 1 | Route JPEG, PNG, and WebP through minimal suffix options so no 8.18-only default fields cross the FFI boundary |
| Final repository scan included a nonexistent optional `compose.yaml` path | 1 | Use the repository's actual `docker-compose.yml`; no build or runtime check was affected |

## Current Task: Resend Email Integration

### Goal

Replace the custom email-provider webhook contract with a direct, production-ready Resend email integration for verification and password-reset messages.

### Phases

**Status:** complete

- [completed] 1. Confirm the current Resend API contract and inventory MediaHub email call sites/configuration
- [completed] 2. Design and implement Resend request mapping and secure configuration
- [completed] 3. Add focused tests for templates, authentication, success, and provider errors
- [completed] 4. Update Compose/example environment and deployment documentation
- [completed] 5. Run formatting, focused tests, workspace checks, and final review

### Current Task Errors

| Error | Attempt | Resolution |
| --- | --- | --- |
| A parallel source scan included nonexistent `crates/mediahub-server/tests` | 1 | Read the existing `src/tests.rs` and `server_config.rs` paths directly; no implementation state was changed |

## Current Task: README Usage and Deployment Guide

### Goal

Rewrite the README's opening guide to document executable startup, production deployment, configuration, storage profiles, and the supported HTTP/WebDAV/S3 protocol boundaries without changing product behavior.

### Phases

**Status:** complete

- [completed] 1. Inventory actual startup commands, environment requirements, storage profiles, and protocol routes
- [completed] 2. Replace the README opening guide with accurate Chinese documentation
- [completed] 3. Verify every documented command and route against repository configuration

### Current Task Errors

| Error | Attempt | Resolution |
| --- | --- | --- |
| The broad `email` test filter also selected an existing SQLx auth-lifecycle test without `DATABASE_URL` | 1 | Use exact `email::tests` and `tests::resend_` filters for unit/provider coverage, then run database-backed tests with the configured PostgreSQL service |
| Docker Desktop stopped before final PostgreSQL/image validation | 1 | Complete non-Docker checks first, then restore the local Docker engine if available and rerun the isolated database/image checks |

## Current Task: Pre-release Crates Quality Review

### Goal

Audit every Rust package under `crates/` before release, prioritizing correctness defects, production risks, architecture and ownership boundaries, module cohesion, and missing regression coverage. This is a review-only task unless the user later requests fixes.

### Phases

**Status:** complete

- [completed] 1. Inventory workspace packages, source layout, tests, and dependency boundaries
- [completed] 2. Run package-level code and architecture reviews in parallel
- [completed] 3. Audit cross-crate contracts, unsafe/error/concurrency patterns, and release configuration
- [completed] 4. Run formatting, compilation, lint, and targeted test verification
- [completed] 5. Corroborate findings and deliver a severity-ordered report with file/line evidence

### Review Rules

- Findings must identify a concrete failure mode or maintainability cost, not merely a style preference.
- Every reported issue must be verified against current source and include an exact file/line reference.
- Existing historical changes are preserved; no product code will be changed during this review.

### Current Task Errors

| Error | Attempt | Resolution |
| --- | --- | --- |
| PowerShell did not expand the `crates/*/Cargo.toml` glob passed to `rg` | 1 | Use `rg` directory scopes or enumerate manifests with `rg --files` for subsequent scans |
| PowerShell also passed `webdav*.rs` and `s3_http*.rs` to `rg` literally | 1 | Use `rg crates/mediahub-server/src -g 'webdav*.rs'` / `-g 's3_http*.rs'` instead of path globs |
| A parallel review batch failed because a no-match `rg` returned exit code 1 | 1 | Re-ran independent reads with per-command error capture; no source output was lost |
| `docker compose ps` could not interpolate required secret variables in the current shell | 1 | Used direct Docker status plus environment-presence checks; no running database or configured destructive-test URL is available |
| One late architecture scan again used a PowerShell path wildcard that `rg` received literally | 1 | Re-ran with the crate directory plus `-g 'lib.rs' -g 'main.rs'`; architecture inventory completed |

## Current Task: Pre-release Crates Remediation

### Goal

Implement and verify the confirmed release findings from the crates quality review. Product code changes are authorized for all crates, adapters, migrations, OpenAPI DTOs, server handlers/workers, and focused regression tests. Preserve existing behavior where it is not part of a finding.

### Phases

**Status:** complete

- [completed] 1. Establish repair matrix and cross-crate interface decisions
- [completed] 2. Repair core/app invariants, upload/session/task recovery, and error semantics
- [completed] 3. Repair image/local/S3 adapter safety and storage contracts
- [completed] 4. Repair PostgreSQL aggregate/tenant/idempotency/webhook boundaries and OpenAPI parity
- [completed] 5. Repair server auth, workers, handlers, and protocol integration
- [completed] 6. Run focused regressions, full formatting/check/Clippy/tests, and final release audit

### Repair Decisions

- Preserve the existing adapter ports where possible; introduce explicit outcome/state variants only when they prevent destructive rollback or lease ambiguity.
- Treat object-store promotion as a durable commit boundary: cleanup failures become retryable orphan cleanup, never evidence that the final object is absent.
- Make durable aggregate operations atomic in the PostgreSQL adapter instead of composing multiple independently committed calls in HTTP handlers.
- Keep public OpenAPI DTOs separate from internal persistence/domain aggregates; never serialize lease tokens or storage internals directly.

### Current Task Errors

| Error | Attempt | Resolution |
| --- | --- | --- |
| Final read-only review found additional upload-reconciliation, 16-bit image, webhook timeout, and OpenAPI timestamp gaps | 1 | Added durable ordinary-upload fencing/heartbeats, byte-accurate image limits, bounded webhook attempts, and explicit public schema contracts; reran the full matrix |
| A combined final-record patch used an error-row anchor from an earlier plan section | 1 | The patch was atomic and changed nothing; split the final updates by file and current-task heading |
# Current Task: Container Entrypoint And Release Tags

## Goal

Ensure the Docker dependency cache cannot package the placeholder server binary, prove the real workspace entry point runs, and publish only the default branch as `master` and `latest`.

## Phases

**Status:** complete

- [completed] 1. Confirm the Cargo workspace/member entry-point contract and diagnose the exit-0 restart loop
- [completed] 2. Fix Docker source invalidation and narrow image metadata tags
- [completed] 3. Build the image and prove the real server process/logging/health behavior
- [completed] 4. Validate workflow syntax and final diffs

## Current Task Errors

| Error | Attempt | Resolution |
| --- | --- | --- |
| Local `actionlint` executable is not installed | 1 | Run the official actionlint container against the workflow instead of installing host tooling |
| Removing version-tag values left an empty workflow `tags:` key | 1 | Delete the empty trigger key and rerun actionlint |
| Dockerfile check timed out fetching Docker Hub auth over IPv6 | 1 | Reuse the locally cached base images with `--pull=false` for the actual image build |
| BuildKit still resolved Docker Hub metadata despite `--pull=false` and hit the same IPv6 timeout | 2 | Switch to the classic local-image builder instead of repeating the BuildKit metadata path |
| The previously used isolated PostgreSQL container no longer exists | 1 | Create a task-owned Docker network and temporary PostgreSQL 17 container, then remove both after the health smoke test |

# Current Task: Unified Web And API Image

## Goal

Build the pnpm Web console inside the deployment image and serve it from the MediaHub Axum process on the same origin without breaking JSON, native object, S3, or WebDAV routes.

The unified image is the only supported Web deployment path; remove the obsolete standalone static-hosting configuration, scripts, dependency, and documentation.

## Phases

**Status:** complete

- [completed] 1. Audit Vite output, API-base behavior, Docker context, and Axum route conflicts
- [completed] 2. Implement same-origin Web defaults and explicit static/SPA serving
- [completed] 3. Add the pnpm Web build stage and runtime assets to the Docker image
- [completed] 4. Update workflow triggers, Compose, and deployment documentation
- [completed] 5. Run Web/Rust tests, build the image, and verify SPA/API/object route behavior

## Current Task Errors

| Error | Attempt | Resolution |
| --- | --- | --- |
| Focused SPA route test could not start because SQLx requires `DATABASE_URL` | 1 | Started a task-owned PostgreSQL instance and reran the compiled integration test with an explicit URL |
| Native-route isolation assertion expected `401`, but an unknown Application correctly returned backend `404` | 1 | Assert the actual not-found contract and retain the response-body check proving the SPA was not served |
| Docker Web-stage build could not fetch the Docker Hub Node token over the host's broken IPv6 route | 1 | Validate the pnpm stage on the host and retry through an accessible registry/cache during final image verification |
| Combined validation aborted after a legitimate no-match `rg` returned exit code 1 | 1 | Treat both obsolete-reference searches as match-tolerant and rerun Actionlint/Compose independently so their outputs are retained |
| First runtime smoke container rejected an invalid fixed Base64 media-signing test key, and its random-port shorthand was not queryable | 1 | Recreate the task-owned container with two generated 32-byte Base64 keys and explicit `127.0.0.1:0:3000` publishing |
| Final evidence command lost quotes inside Docker Go-template and container `stat` arguments | 1 | Runtime HTTP/log checks and cleanup already passed; rerun image metadata inspection with simple independent templates |
