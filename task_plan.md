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

**Status:** in_progress

- [completed] 1. Inventory repository metadata, GitHub Actions, Docker/Compose, and documented deployment contract
- [completed] 2. Audit secrets, licenses, generated artifacts, dependencies, and release-facing configuration
- [completed] 3. Fix confirmed CI, container, dependency, and open-source readiness issues
- [in_progress] 4. Run local CI-equivalent checks and build/test the deployment image
- [pending] 5. Rebuild, inspect the image, and deliver the remaining-risk checklist

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
