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
