# Findings

## Repository shape

- Root src/main.rs is only a placeholder; the production binary is crates/mediahub-server/src/main.rs.
- The production entrypoint was 7,432 lines and about 255 KB before refactoring.
- Existing server support modules include configuration, identity, access keys, runtime storage, S3, and WebDAV.

## Refactor decisions

- main.rs now contains shared imports/constants/state, module declarations, source includes, and a small Tokio entrypoint.
- Responsibility-oriented files:
  - api_error.rs
  - api_types.rs
  - media_http.rs
  - handlers_webhooks.rs
  - handlers_path.rs
  - handlers_media.rs
  - handlers_admin_auth.rs
  - http.rs
- workers.rs is a real child module and exposes only the worker functions/helpers required by startup and handlers.
- bootstrap.rs is a real child module; the binary entrypoint delegates to bootstrap::run.
- The handler implementation files remain crate-level includes for now because they share many private request/response types. This avoids making dozens of DTO fields public as a side effect of the first structural refactor.
## Workspace-wide second pass

- The largest non-test modules were concentrated in the server protocol layer and storage adapters.
- S3 HTTP is now split into core object/list operations, multipart operations, and support/error helpers.
- WebDAV is now split into authentication, guarded filesystem operations, file handles/resources, and support helpers.
- Server handlers are split into health, admin, auth, applications, buckets, media listing, path mutations, async jobs, uploads, path HTTP, media access/auth, webhooks/audit, and access keys.
- API DTOs are split into request DTOs, response DTOs, and shared validation/cursor helpers.
- Local and S3 adapter tests are now outside their production lib.rs files.
- Parent files remain intentionally small include/module facades where the implementation shares private types; this preserves encapsulation while making ownership and navigation explicit.
- PostgreSQL media persistence is split into bucket operations, media/lifecycle queries, media mutations, and support helpers.
- PostgreSQL S3 multipart persistence is split into lifecycle operations and locking/validation/row-conversion helpers.
## Final workspace pass

- In-memory adapters are split by object store, buckets, media, upload sessions, and outbox/clock.
- PostgreSQL control plane is split by auth/session, applications, access keys, and helpers.
- Image adapter is split by libvips, blocking runtime, Rust image pipeline, and tests.
- OpenAPI contract is split by model/helpers, paths, parameters, and components.
- Core async-job domain is split by identity/action, item results, aggregate model, errors/validation, and tests.
- Dedicated database: mediahub_codex_contract in the running mediahub-codex-postgres container.
## Application resource isolation investigation

- The backend client already scopes bucket, media, upload, and Webhook requests with X-MediaHub-App-Id; access keys use an explicit Application path.
- React Query keys already include appId for Buckets, objects, access keys, and Webhooks, so switching causes separate cache entries.
- The Mock implementation is the defect: objects, buckets, accessKeys, webhooks, deliveries, jobs, and uploads are module-global arrays/maps.
- selectedMockApplicationId currently affects only signed URL generation. Resource reads and mutations ignore it, so every Application exposes the same data.
- A newly created Mock Application receives no dedicated resource store, which makes the shared global resources appear immediately.
- The create-application dialog is a single narrow field followed by a full divider and right-aligned actions; it lacks a compact identity cue and balanced spacing.
- Implementation decision: store Mock resources in a Map keyed by public appId; studio keeps demo seeds, marketing/new Applications start empty.
- All Mock mutations must resolve the current Application state, including batch jobs, upload sessions, Webhook deliveries, and key mutations.
- UI decision: use a small dialog, an application identity icon, concise ownership copy, a full-width input, and a compact action footer.
- All mutable global references were enumerated. Admin storage/jobs must aggregate all per-app stores, while normal resource calls use the selected app store.
- Existing pure Mock API tests rely on app_studio as the implicit default, so the state resolver must fall back to the first Application when setApplication has not been called.
- Access-key list/create already receive appId explicitly; update/revoke and all Webhook methods will resolve the selected app store.
- Upload sessions and async jobs are also app-owned and will move into the same per-app state to prevent indirect leakage.
- Regression coverage belongs in App.test.tsx for end-to-end Application switching and in direct Mock API assertions for key/Webhook/Bucket/object isolation.
- ApplicationSwitcher itself already behaves correctly; the visual change belongs to a dedicated create modal component in App.tsx, not the switcher menu.

## Real backend wiring investigation

- The previous browser verification deliberately exercised Mock mode; it did not prove integration with the existing backend.
- The current frontend mode switch treats a missing `VITE_API_BASE_URL` as permission to use Mock data, which is an unsafe default for normal local/deployed runs.
- Intended correction: tests and an explicit Mock flag may use Mock; ordinary development and production runs must resolve a real backend endpoint.
- `web/src/api/client.ts` already defaults the API URL to `http://localhost:3000` and normalizes localhost/127.0.0.1, but `web/src/api/mock.ts` ignores that readiness and selects Mock solely from whether the environment variable exists.
- The backend listens on port 3000 and exposes credentialed CORS/CSRF support; `MEDIAHUB_CORS_ALLOWED_ORIGINS` controls the development console origin allowlist.
- Runtime configuration is concentrated in `docker-compose.yml`, `Dockerfile`, `web/vite.config.ts`, and `web/nginx.local.conf`; there are no checked-in `.env` files.
- `docker-compose.yml` currently starts only the API and PostgreSQL, not the web console, and it does not set `MEDIAHUB_CORS_ALLOWED_ORIGINS`.
- Compose intentionally requires production-grade secret/email variables, so it cannot be used as a zero-configuration local backend without supplying them; this is separate from the frontend accidentally choosing Mock.
- The root `Dockerfile` packages only the Rust server on port 3000; frontend delivery is expected through a separate Vite/Nginx path.
- `web/vite.config.ts` has no development proxy, and `web/nginx.local.conf` only serves SPA assets; therefore the current architecture intentionally uses a direct API origin, defaulting locally to port 3000.
- Because the browser calls port 3000 directly, local development must allow `http://localhost:5173` and/or `http://127.0.0.1:5173` through backend CORS and must keep `credentials: include` for session cookies.
- `backendApi` is already feature-complete for authentication, Applications, Buckets, media, access keys, Webhooks, uploads, jobs, and admin endpoints; replacing it is unnecessary.
- `createMediaHubClient` already sends cookies, CSRF headers on mutations, and `X-MediaHub-App-Id` when an Application is selected.
- The concrete mode-selection defect is only `const useBackendApi = Boolean(configuredApiBaseUrl)`: the resolved default URL exists but is not used to decide the mode.
- Existing Vitest suites import the same API facade and rely on Mock data without setting environment variables, so test mode must remain Mock automatically.
- A minimal compatible policy is: `MODE === 'test'` or `VITE_USE_MOCK_API === 'true'` selects Mock; every other run selects the real backend, with `VITE_API_BASE_URL` overriding the existing localhost:3000 default.
- Backend CORS is credentialed and supports the Application/CSRF headers, but an empty `MEDIAHUB_CORS_ALLOWED_ORIGINS` list permits no cross-origin console requests.
- Backend cookies are secure by default. A direct HTTP local console/API pairing needs `MEDIAHUB_ALLOW_INSECURE_COOKIES=true`; localhost ports are same-site, so the default `SameSite=Lax` remains compatible.
- The runbook already documents the correct local backend flags, including insecure local cookies and a CORS allowlist for the separate Vite origin, but the frontend start path does not enforce or surface them.
- The currently running Vite console has no API base environment configured and is therefore in Mock mode under the old selector.
- `admin@example.com` is only a Mock seed account and its Mock password is `mediahub-admin`, not `admin`; this explains the previously reported `admin@example.com admin` login failure in Mock mode and is not evidence of a real backend admin account.
- Port 3000 is currently backed by Docker/WSL forwarding and returns a MediaHub JSON error envelope, so a real server is already reachable; `/health` is not the correct health route and falls through to the application content route.
- The host process environment has no `VITE_*` or `MEDIAHUB_*` variables, consistent with the frontend having selected Mock while the backend runs separately in Docker.
- User clarified the final requirement: delete all runtime Mock-related code rather than retaining an explicit opt-in or test-mode fallback.
- The production facade should be renamed away from `mock.ts`; demo accounts/data and tests coupled to those seeds must no longer be part of the application runtime.
- `web/src/api/mock.ts` is a 955-line mixed module: shared UI-facing types/API interface occupy lines 11-296, Mock data/implementation occupies lines 297-708, and the real backend adapter occupies lines 709-954.
- Only `App.tsx`, `App.test.tsx`, and `App.variant-preview.test.tsx` import the mixed module, so extraction can be contained to a new real-only API facade plus those three imports.
- The runtime Mock block includes seeded Applications, objects, Buckets, accounts, sessions, jobs, uploads, keys, and Webhooks; deleting lines 297-708 removes the fake backend rather than merely disabling it.
- Most of `App.test.tsx` is an in-process integration suite that directly mutates the runtime Mock API; those cases cannot remain after the fake backend is removed.
- Pure frontend coverage in `App.test.tsx` (path helpers, pagination UI, access-key form, one-time secret behavior) can be retained without a fake backend; backend contracts already have Rust/API tests.
- `App.variant-preview.test.tsx` uses API method spies for a bounded component interaction rather than relying on seeded Mock state, so it can target the real API facade module without carrying a runtime Mock implementation.
- The reusable, backend-independent tests are clearly separable: Object path normalization, directory breadcrumbs, pagination controls, delete-pending cache helpers, access-key form submission, and one-time-secret disposal.
- Auth, Application switching, admin, upload, directory navigation, and object workflow cases in `App.test.tsx` all rely on seeded accounts/resources and should move to real-backend E2E coverage rather than recreate a fake service inside unit tests.
- Mock leakage also exists outside `mock.ts`: the login form is prefilled with `you@example.com`/`mediahub`, multiple route components default missing `appId` to `app_studio`, and Playwright E2E assumes the same seeded account/Application.
- Removing Mock completely therefore requires clearing login defaults, removing the `app_studio` route fallback, and replacing seed-coupled E2E assumptions with environment-supplied real-backend credentials/Application IDs or skipping when they are absent.
- The live Docker deployment already has separate `mediahub-api-docker` (port 3000) and `mediahub-web-local` Nginx (port 5173) containers, plus PostgreSQL on host port 55432.
- Correct backend health endpoints are `/health/live` and `/health/ready`; the earlier `/health` request was expected to miss.
- The running API container is correctly configured for direct browser access: insecure local cookies are enabled and CORS allows both localhost and 127.0.0.1 on port 5173.
- Both live and readiness checks return HTTP 200 with `{\"status\":\"ok\"}`.
- The Nginx container bind-mounts `web/dist`, so rebuilding the frontend in place immediately updates the Docker-served console without adding a frontend image stage.
- The only helper shared across the Mock and real adapter blocks is `lifecycleSummary`; it must be retained during extraction because real Bucket mapping uses it.
- The real adapter closes cleanly before the final conditional export, so the new facade can export `backendApi` directly after removing mode selection.
- The one-time-secret UI exposes a stable accessible close button and a readonly secret input, so E2E assertions do not need the Mock-only `secret_` value prefix.
- Existing E2E paths and credentials are entirely seed-coupled; they will be gated by `MEDIAHUB_E2E_EMAIL`, `MEDIAHUB_E2E_PASSWORD`, and `MEDIAHUB_E2E_APP_ID` and will target that real Application.
- The real-only facade extraction produced `web/src/api/index.ts` with 521 lines and removed the 400+ line in-memory service block plus the conditional mode export.
- After extraction, the only remaining `app_studio` strings are neutral client-test fixture values; they should be renamed so no demo Application identifier remains in the web tree.
- The runbook documents CORS/cookie requirements but the Compose service does not currently pass `MEDIAHUB_CORS_ALLOWED_ORIGINS`, `MEDIAHUB_ALLOW_INSECURE_COOKIES`, or `MEDIAHUB_COOKIE_SAME_SITE` through to the container.
- The product spec already states that Pages inject `VITE_API_BASE_URL`; documentation should now state that there is no standalone/demo data mode and the default local API origin is port 3000.
- The post-removal residue scan found no `api/mock`, Mock mode selector, demo accounts, or demo Application/resource identifiers under the web source tree.
- The first full Vitest run reached the suite successfully but reported three test failures; one is a stale `/营销素材库/` selector after the neutral fixture was renamed to `归档素材库`.
- Focused tests identified the primary failure as a syntax error at `api/index.ts:365`: the generated extraction patch was truncated inside a long `mimeBreakdown`/backend helper span and inserted an ellipsis marker.
- Both focused suites failed during transform before executing tests, so their behavior has not yet been evaluated.
- No application source map/cache contains the deleted mixed source, but inspection shows the corruption is confined to one line between `mimeBreakdown` and the body of `backendObjectById`.
- The missing span is reconstructible from the previously inspected real adapter: MIME breakdown, backend DTO mappers, full media pagination, Bucket/media aggregation, and the `backendObjectById` signature/opening.
- The recovered facade now contains no truncation marker, old Mock import/mode/data identifiers, demo account, or demo Application IDs.
- The remaining ApplicationSwitcher failure is only the selector label still reading `营销素材库` while the fixture now renders `归档素材库`.
- After recovery, focused App/API-facing component suites pass: 3 files and 16 tests.
- TypeScript project compilation passes with the real-only facade.
- Existing HeroUI `PressResponder` test warnings remain informational and are unrelated to backend/Mock removal.
- Full frontend verification passes: 26 Vitest files and 128 tests.
- Production build passes, verifies all 44 OpenAPI paths/67 operations, and rebuilds the Nginx-mounted `web/dist` bundle.
- Existing third-party viewer/CSS/chunk-size build warnings remain unchanged and do not affect the real API wiring.

## Browser verification

- Docker-served `http://127.0.0.1:5173/login` loads successfully with title `MediaHub Console` after rebuilding `web/dist`.
- The live login form contains empty email/password controls and no demo credential hint or prefilled account.
- Submitting `admin@example.com` / `admin` now succeeds against the real backend and navigates to the backend-owned Application `app_019f6607ca6f7471a34fa4f7aa0b22b2` named `代码搬运工`.
- The real dashboard reports 36 objects, one `media` Bucket, 98.0 MB used, local storage capability, and no browser console errors; these values are backend data, not the removed demo seeds.
- The earlier error wait timed out because authentication succeeded and navigation occurred, not because the request stalled.
- The authenticated account currently owns one real Application, so the switcher shows that Application plus the create command; no demo Applications appear.
- The rebuilt `web/dist` contains none of the removed demo account/Application/resource markers or runtime Mock selector symbols.
- Live CORS preflight from `http://127.0.0.1:5173` to the Docker API succeeds with credentials and explicitly allows CSRF/Application headers.
- `docker compose config --quiet` passes with the required deployment secrets supplied, including the new CORS/cookie pass-through entries.
- Playwright discovers the two real-backend workflows; they are credential-gated rather than coupled to removed demo seeds.
- Running Playwright without `MEDIAHUB_E2E_*` credentials exits successfully with both destructive real-backend workflows skipped, as designed.
- Final filesystem verification confirms `web/src/api/mock.ts` is absent and `web/src/api/index.ts` directly exports `backendApi`.

## Open-source release and container audit

### Requirements

- Validate whether the checked-in GitHub Actions workflows build successfully and follow a deployable image publishing contract.
- Build the deployment image locally and inspect the resulting runtime image.
- Review the repository for security, licensing, documentation, packaging, and release-readiness issues before it is made public.
- Fix issues that can be confirmed locally without inventing deployment-specific policy.

### Initial inventory

- The repository has one workflow, `.github/workflows/ci.yml`; it tests Rust and the web console but does not build or publish a container image.
- CI uses PostgreSQL 17, stable Rust with rustfmt/clippy, Node.js 22, `npm ci`, OpenAPI drift checks, unit tests, production web build, libvips tests, and an all-feature Clippy pass.
- The root `Dockerfile` builds only `mediahub-server` with a checksum-pinned libvips source tarball and runs as UID 10001 on Debian Bookworm slim.
- The runtime image exposes port 3000 and persists `/data`; the separate Vite web console is not included in the image or Compose stack.
- Compose requires access-key encryption, media-signing, and email-provider secrets at interpolation time, and persists both PostgreSQL and local object data in named volumes.
- The workspace declares `license = "MIT"`, but the tracked repository has no root `LICENSE` file.
- The repository currently has no `SECURITY.md`, contribution guide, code of conduct, issue templates, or dependency-update configuration.
- Local Docker, Rust, Node.js, npm, and Git are available. `actionlint`, Gitleaks, Trivy, Syft, and Hadolint are not installed locally; containerized or downloaded equivalents may be used for read-only validation.
- The worktree was clean before this audit and the Git remote is `https://github.com/emojiiii/mediahub.git`.

### Documentation and metadata audit

- `readme.md` still labels the project as being in a design phase and functions primarily as a long product specification rather than a public project entry point.
- The README deployment configuration table contains stale generic names such as `STORAGE_BACKEND`, `SESSION_SECRET`, and `MASTER_KEY_V1`; the implementation and Compose file use `MEDIAHUB_*` names documented in `docs/runbook.md`.
- The executable deployment instructions are in the English-only runbook and currently teach source builds (`docker compose up --build`), not pulling a published image.
- The README intentionally documents the web console as a separate Cloudflare Pages/Vite deployment; an API-only container is therefore consistent with the stated architecture, but this must be explicit in the quick start.
- Individual Cargo packages inherit the MIT declaration but generally omit `repository`, `homepage`, `documentation`, `readme`, and sometimes `description` metadata. The web package is private, which is appropriate for an application bundle.
- A tracked-file secret-pattern scan found only Compose/CI development passwords and explicit test fixtures. The single-commit Git history contains no historically tracked `.env`, private-key, credential, or secret-named file.
- Docker Desktop and Buildx are healthy and advertise both `linux/amd64` and `linux/arm64`, so local image and multi-platform build validation are possible.

### Confirmed CI blocker

- `web/package-lock.json` is absent, while the web CI job configures `actions/setup-node` with that exact cache dependency path and runs `npm ci`.
- `npm audit --json` reproduced the underlying failure as `ENOLOCK`; a clean GitHub runner cannot install the web dependencies until a lockfile is committed.
- The missing lockfile also makes dependency resolution non-reproducible and prevents reliable npm vulnerability review.

### Dependency and secret scan

- The generated lockfile initially embedded `registry.npmmirror.com` artifact URLs from the developer-machine npm configuration; a public lockfile should use the canonical npm registry.
- Official npm audit reports 5 vulnerable packages: 4 high and 1 moderate. Vite/esbuild advisories have a supported major-version fix; `xlsx` has prototype-pollution and ReDoS advisories with no npm-registry automatic fix.
- `@open-file-viewer/core` and `@open-file-viewer/react` inherit the `xlsx` advisory, and the project also imports `xlsx` directly for spreadsheet preview. Because uploaded files are untrusted input, this is a real residual risk rather than an unused transitive dependency.
- Gitleaks scanned approximately 2.44 MB and reported one finding. The path/rule still needs to be inspected without exposing the matched value before deciding whether it is a real credential or a documented test placeholder.
- `cargo-audit` and `cargo-deny` are not installed. The attempted `rustsec/rustsec:latest` container fallback is not a published image and failed before scanning.
- The Gitleaks finding is the public example `MEDIAHUB_MEDIA_SIGNING_KEY` in `docs/runbook.md:10`, not a live credential. Replacing fixed example keys with documented random generation will avoid false alarms and teach safer deployment behavior.
- The latest `@open-file-viewer` release (0.1.26) still depends on `xlsx ^0.18.5`; upgrading that package alone does not resolve the SheetJS advisories.
- SheetJS publishes the patched `xlsx` 0.20.3 package from its official CDN, where the package metadata is reachable and declares Apache-2.0. npm `overrides` can force the viewer's transitive copy to this same direct dependency.
- A compatible current frontend toolchain is available: Vite 8.1.5, `@vitejs/plugin-react` 6.0.3, Vitest 4.1.10, and `vite-plugin-static-copy` 4.1.1. All support Vite 8 and Node.js 22+; Vite specifically requires Node 22.12 or newer.

### Container and Rust audit

- Clean `npm ci` succeeds from the new canonical lockfile and official npm audit now reports zero vulnerabilities.
- Actionlint accepts the existing GitHub Actions workflow.
- Hadolint reports only unpinned Debian apt package versions and a missing `pipefail` shell for the checksum pipeline. The checksum pipeline warning is actionable; exact apt patch pinning conflicts with normal Bookworm security updates and will remain an explicitly accepted warning.
- RustSec found two remotely triggerable denial-of-service advisories in `quick-xml 0.40.1`; MediaHub directly iterates checked attributes while parsing untrusted S3 XML, so these are reachable.
- `object_store 0.14.1` updates its `quick-xml` requirement from 0.40.1 to patched 0.41.0. Updating both the workspace dependency and the Server's direct dependency will remove the vulnerable version.
- RustSec also reports the unpatched `rsa 0.9.10` advisory from `Cargo.lock`, but `cargo tree --target all --invert rsa` finds no enabled path in any MediaHub target. Re-evaluate after updating the lockfile; this is not linked into the runtime as currently configured.
- An existing API image proves the runtime volume defect: it runs as UID/GID 10001, `/data` is root-owned mode 0755, `/data/storage` is absent, and a write as the runtime user fails with `Permission denied`.
- `LocalObjectStore::new` creates the configured root at process startup, so a fresh Docker volume prevents local-storage deployments from starting until the image creates and owns `/data/storage` before switching users.

### Implemented release hardening

- Added the missing npm lockfile, upgraded the Vite/Vitest toolchain, and forced direct/transitive SheetJS use to patched 0.20.3; clean install and npm audit both pass.
- Updated `object_store` to 0.14.1 and direct `quick-xml` to 0.41.0; both reachable XML denial-of-service advisories are removed.
- `rsa` remains only beneath the disabled `sqlx-mysql` package recorded by Cargo's lockfile resolver; neither `cargo tree --target all --invert rsa` nor the equivalent sqlx-mysql query finds an enabled target path.
- The image now creates `/data/storage` as UID/GID 10001, provides an HTTP liveness health check, includes CA certificates/curl, enables `pipefail` for source checksum verification, and declares OCI source/license metadata.
- Compose can pull `ghcr.io/emojiiii/mediahub:latest` while retaining a local build definition. PostgreSQL password is now required and public registration defaults to disabled.
- Added a multi-architecture GHCR workflow with PR build-only behavior, branch/tag/SHA labels, BuildKit cache, provenance, and SBOM publication.
- Added Dependabot coverage for Cargo, npm, GitHub Actions, and Docker; added MIT license, security policy, contribution guide, `.env.example`, and secret-safe ignore rules.
- README and runbook now document published-image deployment, the API-only image boundary, secure random key generation, current environment names, and the separate web-console deployment.
- Post-change Actionlint, Compose parsing, Hadolint (excluding intentional apt patch pinning), Gitleaks, and whitespace checks pass.

### Final workflow scope

- The user confirmed that Cloudflare builds the Web UI directly; GitHub Actions must not install, test, or build `web/`.
- Rust CI should run only for `crates/**`, root Cargo manifest/lock changes, or its own workflow definition.
- Container builds should use the same backend paths plus Dockerfile and `.dockerignore` changes. Tag pushes remain release triggers.
- `web/` uses pnpm, and the Docker build context should exclude it completely so UI changes cannot invalidate backend image layers.

### Final verification

- Both GitHub workflows pass Actionlint and contain no Web, Node, npm, or package-lock references.
- `pnpm install --lockfile-only --frozen-lockfile` accepts `web/pnpm-lock.yaml`; Cloudflare can own the actual Web install/build.
- Docker build context is approximately 963 KB with `web/` excluded. `mediahub:open-source-audit` built successfully from Rust 1.88 and custom libvips.
- The final image runs as UID/GID 10001, owns `/data/storage` as mode 0750, can write there through a fresh anonymous volume, has no missing shared libraries, and includes the expected OCI labels and liveness health check.
- A fresh PostgreSQL database plus fresh storage volume reached HTTP 200 on both `/health/live` and `/health/ready`; Docker reported the container as healthy.
- The first runtime attempt correctly failed closed because the reused test database contained object metadata while the fresh storage volume was empty. This was test-state mismatch, not an image defect.
- The first full workspace test run exposed a pre-existing CI failure: `data_plane_sql_keeps_native_types_locks_and_atomic_boundaries` searched only `media.rs` and `s3_multipart.rs`, but both are now include facades and the asserted SQL lives in their split implementation files.
- The SQL invariants themselves remain present. Updating the test input to concatenate the facade plus all included implementation files restores the intended structural coverage without changing database behavior.
- First post-upgrade Clippy run failed only on a current-stable `collapsible_if` lint in `async_job_error.rs`; the validation condition is being collapsed as suggested.
- Vite 8's new Rolldown output omitted the expected `docx-preview` lazy asset and failed `verify-viewer-chunks.mjs`. Vite 7.3.6 remains above all audited Vite vulnerability ranges and preserves the existing Rollup chunk contract, so it is the safer compatibility target.

## Latest dependency upgrade

### Requirements

- Upgrade all direct dependencies where the latest release is compatible after reasonable migration work, with Web UI as the highest priority.
- Use pnpm and keep Cloudflare as the Web build owner; do not reintroduce Web jobs into GitHub Actions.
- Treat latest-major upgrades as migrations that require tests, not automatic version substitutions.
- Record explicit technical reasons for any package that must remain below latest.

### Initial inventory

- The pnpm lock already resolves many caret-ranged Web packages to current releases, but the manifest still advertises older minimums. Latest-major migrations are required for resolvers 5, React Router 7, Zod 4, Vite 8/plugin-react 6, jsdom 29, TypeScript 7, Lucide 1, and openapi-fetch 0.17.
- Current Web latest versions reported by the official registry include Vite 8.1.5, TypeScript 7.0.2, React Router 7.18.1, Zod 4.4.3, Lucide React 1.25.0, jsdom 29.1.1, and `@hookform/resolvers` 5.4.0.
- Cargo can immediately update 38 compatible locked packages on Rust 1.97. Direct major candidates include aes-gcm 0.11, AWS credential/signature crates, hmac 0.13, password-hash 0.6, rand 0.10, reqwest 0.13, sha2 0.11, sqlx 0.9, and tower-http 0.7.
- The production Docker builder is pinned to Rust 1.88 while the current stable toolchain used locally is Rust 1.97.0.
- GitHub Actions also have new majors available for the Docker setup/login/metadata/build actions. These need workflow schema validation after upgrading.

### Direct dependency matrix

- Cargo's non-aggressive direct scan confirms compatible updates for serde 1.0.229, thiserror 2.0.19, uuid 1.24.0, futures 0.3.33, AWS credential types 1.3.0, AWS SigV4 1.5.1, and md-5 0.11.0. An aggressive scan is still required for every cross-major candidate.
- Latest official Actions tags are checkout 7.0.0, setup-qemu 4.2.0, setup-buildx 4.2.0, login 4.4.0, metadata 6.2.0, build-push 7.3.0, and rust-cache 2.9.1.
- The Action versions seen in the screenshot are therefore real current majors rather than Dependabot noise; they can be upgraded together and validated with Actionlint.
- pnpm 11 no longer reads `pnpm.overrides` from package.json and strictly checks the declared package-manager version. The SheetJS security override must move to `web/pnpm-workspace.yaml` before upgrading.
- pnpm 11's first latest update was blocked by supply-chain verification: open-file-viewer 0.1.26 and postcss 8.5.20 were less than 24 hours old, while the external SheetJS 0.20.3 tarball lacked a registry-style integrity entry. No compatibility conclusion can be drawn until the policy is handled explicitly.
- After adding exact release-age/provenance exceptions, pnpm 11 next rejected the fixed SheetJS URL override because `blockExoticSubdeps` forbids URL dependencies below the root. A package-scoped exception is preferable to disabling this policy globally.
- The latest `openapi-typescript` release is 7.13.0 and has no newer preview line; it declares TypeScript `^5.x`. TypeScript 7.0.2 is genuinely incompatible: client generation crashes because `ts.factory` is unavailable through the API shape expected by openapi-typescript.
- The latest compatible TypeScript is 5.9.3, so this is a required compatibility exception rather than an overlooked update.
- The upgraded Web test suite passes: 26 files and 128 tests under pnpm 11.15.0.
- React Router 7 removes the `BrowserRouter.future` prop used to opt into v7 behavior on React Router 6; removing the prop preserves the now-default semantics.
- Vite 8.1.5 successfully compiles the application, but Rolldown 1.1.5 automatically merges the statically imported `docx-preview` dependency into `ObjectFileViewer`. Rolldown's supported `output.codeSplitting.groups` API can preserve the named lazy chunk without reverting Vite.
- After explicit splitting, Vite 8 lists the DOCX chunk in the main chunk's `__vite__mapDeps` table so it can preload it when the lazy `ObjectFileViewer` import is requested. This is not a top-level import and does not load the DOCX code on the initial page.
- `vite-plugin-static-copy` 4.1.1 always preserves matched directory structure unless `rename.stripBase` is enabled. Without migration, PDF.js assets land under `pdfjs/*/node_modules/pdfjs-dist/...` and runtime CMap/font URLs break.
- Official npm audit found five lodash advisories through `open-file-viewer -> mammoth -> argparse@1 -> lodash@3`. Mammoth 1.12.0 does not import argparse anywhere in its JS; it is a stale CLI dependency. Overriding it to latest argparse 3.0.0 removes lodash without changing the browser conversion path.
- After the argparse override, pnpm resolves one argparse 3.0.0 copy for both Mammoth and OpenAPI tooling; lodash is absent, peer checks pass, and official npm audit reports zero known vulnerabilities.
- Stable argon2 0.5.3 still depends on password-hash 0.5. The direct password-hash declaration enables `getrandom` on that same dependency; upgrading it alone to 0.6 would create an unused second type universe and would not upgrade password verification.
- After the Rust migrations, both regular and aggressive cargo-outdated scans report no outdated workspace root dependencies. Libvips 8.18.4 remains the newest stable upstream tag.
- The only Web direct dependency behind `latest` is TypeScript 5.9.3; TypeScript 7.0.2 cannot be used until openapi-typescript publishes compatible support.
- The only Rust direct declaration intentionally below its crate's newest stable major is password-hash 0.5.0, because stable argon2 0.5.3 exposes that exact type line and uses the direct declaration's `getrandom` feature.
- The Rust 1.97.0 deployment image builds successfully on Linux with libvips 8.18.4. Runtime smoke confirms UID 10001, writable storage, healthy HTTP response, and resolved `libvips.so.42` with no missing libraries.

## Libvips CI compatibility

- `libvips` crate 2.3.0 generated `WebpsaveBufferOptions` against libvips 8.18 and always forwards every field to the variadic C API, including `exact=false`.
- The GitHub runner's distro libvips predates the `exact` property, so WebP encoding fails before it can produce output.
- `VipsImage::image_write_to_buffer` accepts saver suffix options and forwards only explicitly named properties. `.webp[Q=...,strip]` preserves quality and metadata stripping without depending on the new `exact` property.
- Debian bookworm currently supplies libvips 8.14.1, making it a suitable lower-bound reproduction environment for the GitHub system-package failure.
- Libvips 8.14.1 also predates the generated binding's `keep` saver property. All three output formats need the minimal suffix-option path, not only WebP.
- The final `.jpg`, `.png`, and `.webp` suffix-option implementation passes all 11 libvips-enabled tests on both Debian libvips 8.14.1 and the deployment image's pinned libvips 8.18.4.
- The release server builds successfully with the pinned library, and the rebuilt deployment image starts as the non-root `mediahub` user with both live and readiness checks returning HTTP 200.

## Resend email integration

- Resend sends mail through `POST https://api.resend.com/emails` with `Authorization: Bearer <API key>` and JSON fields `from`, `to`, `subject`, plus `html` and/or `text`.
- A successful send returns a JSON `id`; API failures use non-2xx status codes and an error object containing `name`, `statusCode`, and `message`.
- Resend accepts an `Idempotency-Key` header up to 256 characters. Keys expire after 24 hours, which can prevent duplicate verification/reset messages during ambiguous retries.
- The default documented rate limit is 10 requests per second per team; `429` responses expose standard rate-limit headers.
- Production senders require a verified Resend domain. The API key must remain server-side and should be scoped to sending where possible.
- MediaHub currently owns its HTTP email client in `mediahub-server`, configured by a provider URL, bearer token, and sender address.
- Registration, resend-verification, and forgot-password handlers all call one `send_token(email, template, token, expires_at)` method; this is the narrow integration boundary to preserve while changing the outbound Resend payload.
- Existing server tests assert the old template-webhook body, so they must be migrated to verify Resend headers, endpoint, rendered subjects/bodies, and provider failure behavior.
- Registration propagates an initial email-delivery failure, while resend-verification and forgot-password deliberately log failures and keep their enumeration-resistant accepted responses. The Resend migration must preserve this behavior.
- Direct sending means MediaHub must now own the verification/reset subject and both HTML/text bodies; the old external provider previously owned template rendering.
- The current provider configuration permits an HTTP endpoint only under an explicit development override. A direct Resend integration can use a fixed HTTPS production endpoint while retaining an injectable endpoint only for local tests.
- The Web console already consumes `/verify-email?token=...` and `/reset-password?token=...`; direct email rendering therefore needs a configured public console origin so messages can contain actionable links.
- No public console URL exists in the backend configuration today. It should be explicit rather than inferred from CORS, because CORS may contain multiple origins or be empty.
- The existing server already depends on `reqwest`, `serde`, `serde_json`, `time`, `url`, and SHA-256 utilities, so the Resend integration does not require a new SDK or dependency.
- Compose and `.env.example` currently expose the generic provider URL/token contract; these should become Resend-specific API key plus a public Web URL while retaining the verified sender setting.
- The implementation uses the existing Web routes, a fixed Resend HTTPS endpoint, a 10-second request timeout, and a token-hash idempotency key. It validates the Resend response ID before treating a send as accepted.
- Runtime configuration now requires `MEDIAHUB_RESEND_API_KEY`, a verified `MEDIAHUB_EMAIL_FROM`, and a clean HTTPS `MEDIAHUB_WEB_URL`; the development-only exposed-token mode can still run without Resend.

## README deployment documentation

- The repository's published image contains the API and background workers only; the React Web console is a separate pnpm/Cloudflare Pages deployment configured with `VITE_API_BASE_URL`.
- The implemented HTTP surface has four distinct entry points: JSON control-plane routes under `/api/v1/*`, native path-style object routes under `/{app_id}/...`, WebDAV under `/dav/{app_id}/...`, and a bounded S3 gateway under `/s3/{bucket}/{object_key}`.
- The S3 gateway supports the PutObject, presigned/header-signed GetObject, and HeadObject operations needed by the documented sub2api integration; it is intentionally not a full S3-compatible administration/API implementation.
- Docker Compose persists PostgreSQL metadata in `mediahub-postgres-data` and Local object bytes in `mediahub-data`; S3 mode stores bytes in the configured external backend while retaining PostgreSQL metadata locally.
- The compose file also includes a `build` target, but prebuilt deployments should run from the repository root with `docker compose pull mediahub` and `docker compose up -d --no-build`; source builds use `docker compose up -d --build`.
- The API router confirms the S3 gateway supports GET/HEAD/PUT/POST/DELETE at `/s3/{bucket}/{object_key}` plus bucket listing/POST at `/s3/{bucket}`, while WebDAV is mounted at `/dav`, `/dav/`, and `/dav/{*path}`.
- The existing runbook contains the authoritative backup, PostgreSQL, S3, WebDAV, HMAC, and verification details; the README should be a concise operational front door linking to it rather than duplicating every contract.
- README review found two historical phrases that described S3 and email as future/configurable provider features; the implementation now documents S3 as supported and Resend as the concrete email service.
