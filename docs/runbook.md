# Local Runbook

This repository's `readme.md` is the product design specification. This file
documents the currently implemented local Docker-profile foundation.

## Run

```powershell
function New-MediaHubKey {
    $bytes = [byte[]]::new(32)
    [System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
    [Convert]::ToBase64String($bytes)
}
$env:MEDIAHUB_ACCESS_KEY_MASTER_KEY = New-MediaHubKey
$env:MEDIAHUB_MEDIA_SIGNING_KEY = New-MediaHubKey
$env:MEDIAHUB_ALLOW_INSECURE_COOKIES = 'true'
$env:MEDIAHUB_CORS_ALLOWED_ORIGINS = 'http://localhost:5173,http://127.0.0.1:5173'
$env:MEDIAHUB_EXPOSE_AUTH_TOKENS = 'true' # isolated local development only
$env:MEDIAHUB_REGISTRATION_ENABLED = 'true'
$env:MEDIAHUB_STORAGE_BACKEND = 'local'
$env:MEDIAHUB_POSTGRES_PASSWORD = 'mediahub-local-only' # isolated local development only
$env:MEDIAHUB_DATABASE_URL = 'postgres://mediahub:mediahub-local-only@127.0.0.1:5432/mediahub'
docker compose up -d postgres
cargo run -p mediahub-server
```

The default URL is `http://127.0.0.1:3000`. Metadata is stored in PostgreSQL 17;
object files are stored under `data/storage`.

For a container deployment, copy `.env.example` to `.env`, fill every required
secret/provider value, and pull the published API/worker image:

```powershell
Copy-Item .env.example .env
docker compose pull
docker compose up -d --no-build
docker compose ps
```

The default image is `ghcr.io/emojiiii/mediahub:latest`. Set `MEDIAHUB_IMAGE`
to a version tag or digest for reproducible deployments. The image contains the
API and workers only; deploy the `web/` console separately and set
`VITE_API_BASE_URL` to the public API origin.

The web console has no embedded demo or Mock API. It always calls the real API,
using `VITE_API_BASE_URL` when set and `http://localhost:3000` otherwise. Start
the local console in a second terminal with:

```powershell
Set-Location web
pnpm install --frozen-lockfile
pnpm dev
```

```powershell
$env:MEDIAHUB_EMAIL_PROVIDER_URL = 'https://mail.example.com/mediahub/tokens'
$env:MEDIAHUB_EMAIL_PROVIDER_TOKEN = '<provider-bearer-token>'
$env:MEDIAHUB_EMAIL_FROM = 'mediahub@example.com'
$env:MEDIAHUB_CORS_ALLOWED_ORIGINS = 'https://console.example.com'
docker compose up --build
```

The Docker profile persists PostgreSQL in `mediahub-postgres-data` and Local
Storage objects in `mediahub-data`. Terminate TLS before exposing the service,
because browser session cookies are `Secure`.
The Compose profile requires a real HTTPS email Provider; the example values
above are placeholders and must be replaced before registration or password
reset can deliver a token.

## Implemented API

The versioned contract is [openapi.json](../openapi/openapi.json).

- `POST /api/v1/auth/register`, `verify-email`, `resend-verification`, `login`, `logout`, `forgot-password`, and `reset-password`; `GET /api/v1/auth/me`
- `GET`/`DELETE /api/v1/auth/sessions`; `DELETE /api/v1/auth/sessions/{session_id}`
- `GET`/`POST /api/v1/applications`; `GET`/`PATCH`/`DELETE /api/v1/applications/{app_id}`
- `GET`/`POST /api/v1/webhooks`; `PATCH`/`DELETE /api/v1/webhooks/{webhook_id}`
- `GET /api/v1/webhooks/{webhook_id}/deliveries`; `POST .../{event_id}/replay`
- `GET`/`POST /api/v1/buckets`; `GET`/`PATCH`/`DELETE /api/v1/buckets/{name}`
- `POST /api/v1/media/batch`; `GET`/`DELETE /api/v1/jobs/{job_id}`
- cursor-filtered and delimiter-aware `GET /api/v1/media`, plus multipart `POST /api/v1/media`
- `POST /api/v1/uploads`; capability-protected `PUT /api/v1/uploads/{upload_session_id}/content`; authenticated `POST .../complete` and `DELETE /api/v1/uploads/{upload_session_id}`
- authenticated `GET /api/v1/uploads/{upload_session_id}` for durable state and pending Local PUT target refresh
- `GET /{app_id}` plus `GET`/`HEAD`/`PUT`/`DELETE /{app_id}/{bucket}` for path-based Bucket/object discovery and empty-Bucket management
- `GET`/`HEAD`/`PUT`/`PATCH`/`POST`/`DELETE /{app_id}/{bucket}/{object_key}` for immutable content, Metadata, signed URLs, and scheduled deletion
- `/dav/{app_id}/{bucket}/...` for an Access-Key-authenticated WebDAV view of the same durable objects
- `GET /api/v1/audit-logs` for recent application-scoped management events
- `GET`/`POST /api/v1/applications/{app_id}/access-keys`; `PATCH`/`DELETE /api/v1/access-keys/{access_key_id}`
- admin-only global user, Application, Job, storage, and audit endpoints under `/api/v1/admin/*`
- `/health/live`, `/health/ready`, `/api/v1/capabilities`, and protected Prometheus `/metrics`

`MEDIAHUB_ACCESS_KEY_MASTER_KEY` is required and must be a base64-encoded
32-byte AES-256-GCM key. Access Key secrets are displayed only by the create
response and stored encrypted at rest. `MEDIAHUB_ACCESS_KEY_MASTER_KEY_VERSION`
defaults to `1`. During rotation, retain old keys with
`MEDIAHUB_ACCESS_KEY_MASTER_KEYRING=1:old-key,2:older-key` while the primary
`MEDIAHUB_ACCESS_KEY_MASTER_KEY` holds the active version's key.
Startup scans every Access Key and Webhook ciphertext key version before
binding the listener. If any referenced historical version is absent from the
configured keyring, startup fails closed instead of reporting ready while
background delivery cannot decrypt secrets.

`MEDIAHUB_MEDIA_SIGNING_KEY` is a separate base64-encoded key of at least 32
bytes. It signs short-lived private-media URLs and upload-content capability
URLs and is required at startup; do not reuse the Access Key encryption key.

New accounts remain `pending_verification` until `verify-email` atomically
consumes their short-lived token. Password reset tokens are likewise hashed at
rest, expire quickly, and can be consumed only once; a successful reset revokes
every existing Session in the same transaction. Authentication endpoints apply
IP and hashed-subject rate limits. Public registration is enabled by default;
set `MEDIAHUB_REGISTRATION_ENABLED=false` to reject every registration attempt
with `403 registration_disabled` without parsing account-specific input.
Production responses never include raw
verification/reset tokens. `MEDIAHUB_EXPOSE_AUTH_TOKENS=true` exists only for
isolated local development and automated tests and must not be enabled on a
shared or Internet-facing deployment.

To bootstrap the first system administrator, register and verify the account,
then start once with `MEDIAHUB_BOOTSTRAP_ADMIN_EMAIL` set to its email address.
The promotion, deployment marker, and audit event commit atomically. Immediately
remove the environment variable after that successful start: if the completed
deployment is restarted while the variable remains set, startup fails closed.
Bootstrap also refuses to run when an administrator already exists.

`/metrics` exposes global operational and storage data and is therefore not
anonymous. It accepts an active administrator Session or a dedicated
`Authorization: Bearer` credential configured with
`MEDIAHUB_METRICS_BEARER_TOKEN`. The token must contain 32-512 printable bytes;
keep it in deployment secret management. Metrics include HTTP throughput,
errors and aggregate latency, uploaded bytes, Variant cache hits/misses,
AsyncJob/deletion/Webhook backlog, quota usage, object/Variant usage, and
physical disk capacity/availability.

Cookie-authenticated API requests select the oldest Application by default.
Send `X-MediaHub-App-Id` with an owned public `app_id` to operate on another
Application. Access Key HMAC credentials remain fixed to their own Application
and cannot use this header to switch context.

Production startup requires `MEDIAHUB_EMAIL_PROVIDER_URL` (HTTPS),
`MEDIAHUB_EMAIL_PROVIDER_TOKEN`, and `MEDIAHUB_EMAIL_FROM`. The server sends a
Bearer-authenticated JSON request containing `from`, `to`, `template`, `token`,
and `expires_at`; templates are `verify_email` and `reset_password`. Raw tokens
remain absent from the database. Plain HTTP is accepted only when
`MEDIAHUB_ALLOW_INSECURE_EMAIL_PROVIDER=true` is explicitly set for isolated
local testing. If no provider is configured, startup is allowed only with the
development-only `MEDIAHUB_EXPOSE_AUTH_TOKENS=true` switch.

Webhook endpoint secrets are returned only by endpoint creation or explicit
rotation responses, then persisted with the same versioned keyring. Outbox
events atomically fan out into independently leased Endpoint deliveries. Each
request includes stable `X-MediaHub-Event-Id`, `X-MediaHub-Event-Type`,
timestamp, and `v1=<hmac-sha256>` signature headers. Failures use bounded
exponential backoff and move to dead-letter after eight attempts, so one broken
Endpoint cannot block another subscriber. Endpoint URLs reject localhost,
private, link-local, mapped-private, documentation, and reserved addresses.
Delivery disables redirects and pins the validated DNS result to the outgoing
connection to close redirect and DNS-rebinding paths.

Webhook delivery history is Endpoint- and Application-scoped. The history page
reports pending/delivered/dead-lettered state, attempt count, last HTTP status,
last transport/error summary, replay count, and relevant timestamps. A manual
replay is accepted only for a terminal delivery, preserves the original Event
ID, clears its terminal queue state, and increments `replay_count`. Receivers
must continue deduplicating by Event ID.

For HMAC, build the canonical request as uppercase method, path, sorted and
form-percent-encoded query pairs, sorted signed headers, SHA-256 body hex,
RFC3339 UTC timestamp, nonce, and idempotency key, each on its own line. Send
the exact signed header names in `Authorization: MH-HMAC-SHA256
SignedHeaders=...; Signature=...`. Mutation requests require a fresh signed
`X-MediaHub-Nonce`; reusing it returns `401`.

HMAC `POST /api/v1/buckets` and `POST /api/v1/uploads` require a signed
`Idempotency-Key`. Retry with a new nonce and the same request to receive the
original `201` response; reusing that key for different request content returns
`409`. UploadSession creation commits its quota reservation, Session state, and
stored response in one transaction.

`POST /api/v1/media` accepts the canonical multipart field `bucket` (the
Bucket name), an optional `object_key`, optional positive `ttl_seconds`, and
`file`. When no object key is provided the server generates an immutable
`uploads/<uuid>` key. Media type is inferred from file bytes, rather than
trusting the multipart `Content-Type` declaration.

For a durable, storage-backend-independent upload lifecycle, first send
`POST /api/v1/uploads` with the Bucket name in `bucket`, a positive
`expected_size`, and `content_type`. Optional object fields are `object_key`,
`original_name`, `display_name`, `extension`, `visibility`, a positive
`ttl_seconds`, and namespaced `metadata`. Creation reserves quota and returns
`upload_id`, the future `media_id`, normalized expectations, and a short-lived
`PUT` target with required headers.

Send the complete body to the returned URL, including its `token` query and
the exact `Content-Length` and normalized `Content-Type` declared at creation.
The URL is an anonymous capability bound to `PUT`, its UploadSession ID, and
its expiry. Treat the entire URL as a secret: do not persist it or include it
in application, proxy, analytics, or error logs. A missing, expired, tampered,
or session-mismatched token is deliberately reported as `404`. Repeating a
byte-identical PUT returns `204`; different content at the same target returns
`409`.

Finally send `POST /api/v1/uploads/{upload_session_id}/complete` with the
client-computed full-body `sha256`. Completion requires Session/HMAC identity
with `media:upload`; cookie-authenticated requests also require CSRF. The
server independently verifies size, media type, and SHA-256 before atomically
activating Media, converting reserved quota to used quota, and creating the
Outbox event. The first completion returns `201`; replay returns the same Media
and Event ID with `already_completed: true` and status `200`. Authenticated
`DELETE /api/v1/uploads/{upload_session_id}` idempotently cancels a pending
session, deletes temporary content, and releases its quota reservation with
`204`. The lifecycle worker applies the same cleanup to expired sessions.

The Local Profile target is one complete, idempotent PUT. It does not accept
ranges, append chunks, or multipart part completion, so
`GET /api/v1/capabilities` intentionally continues to report
`resumable_upload: false`.

Image reads accept `w`, `h`, `fit`, `quality`, `format`, `blur`, `crop`, and
`background`. When both `w` and `h` are omitted, the processor preserves the
source dimensions; providing one dimension infers the other from the source
aspect ratio. Defaults are `fit=inside`, `quality=80`, `format=webp`, `blur=0`,
`crop=center`, and `background=ffffff`. The processor limits input bytes,
decoded allocation, source dimensions, output dimensions, output pixels, and
concurrent generation. Cache identity binds the original SHA-256, normalized
parameters, and processor version. Variant files are staged and atomically
promoted; a database lease and unique `(media_id, transform_key)` constraint
prevent duplicate publication. Private access and cache lifetime inherit the
original Media. Range is ignored for transformed reads, which always return a
full `200` response and `Accept-Ranges: none`.

Bucket object URLs are stable relative paths and do not require MediaHub to
manage a deployment domain. For example, an object with key
`campaigns/2026/logo.png` is available at:

```text
/app_0123456789abcdef/assets/campaigns/2026/logo.png
```

Place the deployment domain in front of that path after configuring the Docker
reverse proxy. `app_id` is required because Bucket names are unique only within
an Application. Public objects use this path directly. Private objects use the
same path with the short-lived `token` returned by `POST` on the object path.
A correctly signed HMAC request with `media:read` may also read a private
object; browser Sessions do not implicitly authorize content URLs. The route
returns `404` for missing, non-active, or unauthorized private objects. An
object visibility override takes precedence over the Bucket default.

WebDAV is mounted at `/dav`. Use the Access Key ID as the Basic username and
the one-time Access Key Secret as the password:

```text
/dav/
/dav/{app_id}/
/dav/{app_id}/{bucket}/
/dav/{app_id}/{bucket}/{object_key}
```

The root exposes only the credential's own Application. Buckets are durable
collections; Object Key prefixes are virtual directories. `PROPFIND`,
`GET`/`HEAD`/Range, bounded immutable `PUT`, `MKCOL`, `COPY`, `MOVE`, `DELETE`,
and lock discovery are supported. A repeated PUT to an existing Object Key is
a conflict, and deleting an object schedules the normal asynchronous deletion
workflow. Bucket deletion remains empty-only. WebDAV operations use the same
PostgreSQL repositories, quota and Bucket policy checks, Local/S3
ObjectStore, transactional Outbox, and audit trail as the JSON API; `/dav`
never exposes the Local storage directory directly.

The ordinary cross-platform development build uses the bounded Rust `image`
processor so it can run without native packages. The Dockerfile installs
libvips, enables `mediahub-server/docker-libvips`, and runs the Docker Profile
with `VipsImageProcessor`. CI installs the native development package, runs the
libvips format/dimension tests, and type-checks the feature-enabled Server.
Each processor reports a distinct version, so cached Variants are never reused
across implementations.

`POST /api/v1/media/batch` requires `Idempotency-Key` and one explicit,
duplicate-free Media ID list. The supported actions are `update_ttl_seconds`,
`update_visibility`, and `delete`. Admission verifies that every target belongs
to the current Application before changing any item. Batches of at most 25
execute synchronously and return ordered per-item success/failure records; the
durable idempotency record replays those exact response bytes. Larger requests
return a durable `202` AsyncJob. `GET /api/v1/jobs/{job_id}` returns ordered
item results and `DELETE` cancels only pending/running work. Workers claim jobs
with expiring lease tokens. Retries are replay-safe: TTL is anchored to Job
creation time, equal visibility/TTL values are no-ops, and delete accepts an
already pending/deleted target.

Bucket create/update accepts up to 32 `lifecycle_rules`. `expire_after` uses a
stable Application/Bucket/Object-Key-Prefix scope and positive
`duration_seconds`; `keep_latest` retains the newest positive `count` using
`created_at DESC, id DESC`. The worker repeatedly scans bounded candidate
batches and schedules the existing idempotent delete workflow with reason
`ttl` or `keep_latest`. Physical deletion removes all Variant binaries before
the original, then transactionally deletes Variant rows, scrubs user/AI
Metadata from the tombstone, releases quota, and appends `media.deleted`.

`GET /api/v1/media` returns `{items,common_prefixes,next_cursor}`. Without a
delimiter it is ordered by `created_at DESC, id DESC`. Optional filters are
`bucket`, `status`, exact `mime`, RFC3339 `created_from`/`created_before`, and
Object Key `prefix`. With one Bucket and `delimiter=/`, it returns only direct
objects plus deduplicated direct-child prefixes, ordered with folders first and
then Object Key ascending. `limit` is 1 through 100 across both entry kinds.
Cursors are opaque, mode-specific, and must be replayed with the same filters;
they never weaken the current Application scope.

## PostgreSQL Repository

`crates/mediahub-adapter-postgres` implements the shared Bucket/Media/Quota,
UploadSession, Outbox/Webhook delivery, AsyncJob, and Variant repository
contracts with native UUID/TIMESTAMPTZ/JSONB columns, advisory locks, and
`FOR UPDATE SKIP LOCKED` claims. Its embedded migrations also create the
control-plane tables and constraints required by authentication, sessions,
one-time tokens, access keys, replay protection, idempotency, audit, and
system administration. The Server uses PostgreSQL for the control-plane,
data-plane, worker, lifecycle, and Webhook repositories.

Start the required local PostgreSQL 17 service without starting the API:

```powershell
docker compose up -d postgres
docker compose ps postgres
```

The service binds PostgreSQL only to `127.0.0.1` and uses
`mediahub-local-only` as a development-only default password. Override
`MEDIAHUB_POSTGRES_DB`, `MEDIAHUB_POSTGRES_USER`,
`MEDIAHUB_POSTGRES_PASSWORD`, and `MEDIAHUB_POSTGRES_PORT` when needed.

Run the real migration and repository contract suite against this isolated
database:

```powershell
docker compose exec postgres createdb -U mediahub mediahub_contract
$env:MEDIAHUB_TEST_POSTGRES_URL = "postgres://mediahub:mediahub-local-only@127.0.0.1:5432/mediahub_contract"
cargo test -p mediahub-adapter-postgres --test repository_contract
```

The contract database must not contain unrelated data: the suite truncates
`users` with `CASCADE`. On a database that already has migration `0001`, the
same command exercises the remaining embedded control-plane/runtime upgrades.
Create `mediahub_contract` only once and reserve it exclusively for tests.
`MEDIAHUB_TEST_POSTGRES_URL` is the explicit destructive-test gate. The
contract test fails before connecting when it is unset, and CI always supplies
an isolated PostgreSQL 17 database.

`MEDIAHUB_DATABASE_URL` must use a `postgres:` or `postgresql:` URL. The Server
connects to PostgreSQL and applies its embedded migrations before workers or the
listener start. Unknown schemes are rejected, and a failed connection stops
startup.

Compose supplies the PostgreSQL service hostname to the API and waits for the
database health check before starting MediaHub:

```powershell
docker compose up -d --build
```

Compose requires an explicit PostgreSQL password. Override every development
credential above for any shared deployment. Public registration defaults to
disabled unless `MEDIAHUB_REGISTRATION_ENABLED=true` is set deliberately.
PostgreSQL backup/restore uses database-native point-in-time recovery together
with the object-storage snapshot boundary.

## S3-Compatible ObjectStore Profile

`crates/mediahub-adapter-s3` implements the shared immutable `ObjectStore`
contract and the direct-upload `UploadSessionStorage` contract over an
S3-compatible backend. Direct uploads use AWS SigV4 query presigning and bind
the exact object key, `PUT`, `Content-Length`, `Content-Type`, and a maximum
15-minute expiry. Completion performs `HEAD` followed by an ETag/version-fenced
streaming `GET`, reads stored Content-Type metadata, and calculates SHA-256 over
the bytes; an ETag is never treated as the content checksum. The package's
default tests use an in-memory backend and deterministic signing assertions so
overwrite protection, retry convergence, inspection, and idempotent deletion
run in every workspace test:

```powershell
cargo test -p mediahub-adapter-s3 --test object_store_contract
```

The Server selects storage independently from its PostgreSQL database
profile. To run the Compose API against an external S3-compatible service,
configure the bucket and endpoint before starting it:

```powershell
$env:MEDIAHUB_STORAGE_BACKEND = 's3'
$env:MEDIAHUB_S3_BUCKET = 'mediahub-production'
$env:MEDIAHUB_S3_REGION = 'us-east-1'
$env:MEDIAHUB_S3_ENDPOINT = 'https://s3.example.com' # omit for AWS S3
$env:MEDIAHUB_S3_ACCESS_KEY_ID = '<access-key>'
$env:MEDIAHUB_S3_SECRET_ACCESS_KEY = '<secret-key>'
$env:MEDIAHUB_S3_SESSION_TOKEN = '<temporary-token>' # optional
$env:MEDIAHUB_S3_PREFIX = 'mediahub' # optional object-key namespace
$env:MEDIAHUB_S3_VIRTUAL_HOSTED_STYLE = 'false'
$env:MEDIAHUB_S3_ALLOW_HTTP = 'false'
docker compose up -d --build
```

`MEDIAHUB_S3_ACCESS_KEY_ID` and `MEDIAHUB_S3_SECRET_ACCESS_KEY` must be set
together. `MEDIAHUB_S3_REGION` defaults to `us-east-1`; endpoint, session token,
and prefix are optional. Use `MEDIAHUB_S3_ALLOW_HTTP=true` only for an isolated
local compatible service. Production endpoints must use HTTPS. Set
`MEDIAHUB_S3_VIRTUAL_HOSTED_STYLE=true` only when the provider expects the
bucket in the hostname rather than the request path.

Readiness verifies the selected object store. Admin storage and Prometheus
continue to report database object/Variant usage for S3, but
`disk_total_bytes` and `disk_available_bytes` are both `0`: remote bucket
capacity is not a local filesystem fact and MediaHub does not invent one.

Run the same contract against an isolated real bucket explicitly. The test
creates a unique prefix but must never target a bucket containing unrelated
objects:

```powershell
$env:MEDIAHUB_TEST_S3_BUCKET = 'mediahub-contract-test'
$env:MEDIAHUB_TEST_S3_REGION = 'us-east-1' # optional; this is the default
$env:MEDIAHUB_TEST_S3_ENDPOINT = 'https://s3.example.com' # optional for AWS
$env:MEDIAHUB_TEST_S3_ACCESS_KEY_ID = '<test-access-key>' # optional when the AWS credential chain is configured
$env:MEDIAHUB_TEST_S3_SECRET_ACCESS_KEY = '<test-secret-key>'
$env:MEDIAHUB_TEST_S3_SESSION_TOKEN = '<temporary-session-token>' # optional
cargo test -p mediahub-adapter-s3 --test object_store_contract `
  real_s3_satisfies_shared_object_store_contract -- --ignored --exact
```

The ignored real-backend contract also sends a presigned PUT with the required
headers, independently inspects its size, MIME, and SHA-256, then aborts it
twice. This V1 target is a single complete PUT and creates no multipart upload
state. If multipart support is added later, persist its upload ID and terminate
it before deleting the object during cancellation or expiry.
This test remains `#[ignore]` during ordinary workspace runs and requires
`MEDIAHUB_TEST_S3_BUCKET`; the remaining `MEDIAHUB_TEST_S3_*` variables select
the endpoint and credentials. Without the explicit ignored invocation and an
isolated bucket, no real S3 contract has run.

The S3 runtime uses the same immutable read/write/delete, direct-upload, and
Variant paths as Local storage. `MEDIAHUB_STORAGE_ROOT` and the Compose data
volume remain relevant to the default Local profile; S3 object bytes are not
written into that directory.

## Inbound S3 Gateway For sub2api

The outbound ObjectStore profile above controls where MediaHub stores bytes.
The inbound gateway at `/s3/{bucket}/{object_key}` instead lets a bounded AWS
SDK client store normal MediaHub objects. It supports the `PutObject`,
presigned/header-signed `GetObject`, and `HeadObject` operations required by
`sub2api` asynchronous image storage. It does not claim general S3 API
compatibility.

Use an Application AccessKey with `media:upload` and `media:read` as the S3
credential pair. The AccessKey identifies the Application; Bucket is the
path-style S3 Bucket. Configure clients with the public MediaHub origin plus
`/s3`, `force_path_style=true`, an arbitrary consistent Region such as
`us-east-1`, and no `public_base_url` for private Buckets. The complete
configuration and proxy requirements are in
[`sub2api-async-image-storage.md`](sub2api-async-image-storage.md).

The gateway aggregates one signed PUT up to 64 MiB and then delegates to the
same immutable upload application service as the native API. The result is a
Media record with quota, Bucket policy, lifecycle, audit, outbox, and storage
rollback behavior. Reads use the standard MediaHub Range/ETag/throttling path.
AWS XML errors are intentionally outside the JSON control-plane OpenAPI; use
`GET /api/v1/capabilities` and require `s3_gateway=true` for discovery.

Every response includes `X-Request-Id`. A syntactically valid caller-provided
value (up to 128 letters, digits, `_`, or `-`) is echoed; otherwise the service
generates one. JSON errors also include `error.request_id`.

Management mutations create append-only audit events for the current
Application. Audit summaries contain only non-sensitive resource facts and
never contain SecretAccessKeys, sessions, signatures, prompts, or file bytes.

For a separate frontend origin, set `MEDIAHUB_CORS_ALLOWED_ORIGINS` to a
comma-separated exact allowlist, for example `http://localhost:5173`. The API
allows credentialed CORS only for this list, including `X-CSRF-Token`. For
cross-site frontend cookies also set `MEDIAHUB_COOKIE_SAME_SITE=none` while
keeping secure cookies enabled. Local HTTP development instead uses
`MEDIAHUB_ALLOW_INSECURE_COOKIES=true` and the default `SameSite=Lax`.
The Compose service passes all three settings through to the API container.

## PostgreSQL Backup And Restore

Treat PostgreSQL metadata, object storage, the keyring, and deployment secrets
as one recoverable dataset. Production deployments should use continuous WAL
archiving, tested point-in-time recovery, and the storage provider's versioning
or snapshot facility at a recorded consistency boundary. A logical `pg_dump`
is useful for migration rehearsal, but it is not a replacement for PITR.

Before a coordinated offline snapshot, stop the API and workers while leaving
PostgreSQL running, record the database and storage snapshot identifiers, and
retain every master-key version referenced by encrypted rows. Restore into an
isolated environment first, apply WAL to the chosen recovery point, restore the
matching object snapshot, and only then start MediaHub.

After restore, require PostgreSQL readiness and application readiness, compare
database and object inventory totals, sample object hashes, and verify Session,
HMAC, signed-media URL, and Webhook signing flows before exposing traffic. The
repository currently does not ship an automatic metadata/object repair tool;
operators must preserve evidence and reconcile discrepancies explicitly.

## Verify

```powershell
cargo fmt --check
docker compose up -d postgres
$env:MEDIAHUB_TEST_POSTGRES_URL = 'postgres://mediahub:mediahub-local-only@127.0.0.1:5432/mediahub_contract'
$env:DATABASE_URL = $env:MEDIAHUB_TEST_POSTGRES_URL # Required by sqlx::test server tests.
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
Set-Location web; pnpm build
```

The current Server supports runtime Local/S3 selection, but intentionally does
not claim resumable multipart uploads. Rust DTOs and the contract table generate
`openapi/openapi.json`; TypeScript declarations are generated from that document,
consumed through the typed runtime client, and checked for drift by CI.
