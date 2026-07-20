# MediaHub

> 面向 AI 时代媒体产物的自托管对象存储与处理服务。

当前状态：正式版
文档版本：v1.0

## 快速开始

MediaHub 的发布镜像包含 Web 控制台、API 和后台 Worker，由同一个 Axum 服务在同一 Origin 提供。服务器只需要 Docker Engine 和 Docker Compose v2，不需要单独部署前端。

### 使用已发布镜像部署服务器

这是生产环境推荐方式。GitHub Actions 使用 pnpm 构建 Web、编译 Rust 和 libvips，并发布 `linux/amd64` 镜像；服务器不需要安装 Node.js、pnpm 或 Rust。

在服务器上执行，且必须位于包含 `docker-compose.yml` 的仓库根目录：

```bash
git clone https://github.com/emojiiii/mediahub.git /opt/mediahub
cd /opt/mediahub
cp .env.example .env
chmod 600 .env
```

编辑 `.env`，填写所有必填密钥、Resend 和 MediaHub 公网 HTTPS Origin，然后启动：

```bash
docker compose config
docker compose pull mediahub
docker compose up -d --no-build
docker compose ps
curl --fail http://127.0.0.1:3000/health/ready
```

启动成功后，直接访问反向代理绑定的 MediaHub 域名即可打开 Web 控制台，例如 `https://mediahub.example.com/`。

`docker compose config` 用于提前检查 Compose 文件和必填环境变量。不要在 `web/`、家目录或其他没有 `docker-compose.yml` 的目录执行 Compose 命令。

默认镜像为 `ghcr.io/emojiiii/mediahub:latest`。工作流只发布 `latest` 和 `master`；需要严格复现时应固定镜像摘要：

```dotenv
MEDIAHUB_IMAGE=ghcr.io/emojiiii/mediahub@sha256:替换为实际摘要
```

如果 GHCR 包是私有的，先使用具有 `read:packages` 权限的 Token 登录：

```bash
echo "$GHCR_TOKEN" | docker login ghcr.io -u YOUR_GITHUB_USERNAME --password-stdin
```

更新已运行的服务器：

```bash
cd /opt/mediahub
docker compose pull mediahub
docker compose up -d --no-build
docker compose logs --tail=100 mediahub
```

### 从源码构建服务器

只有在服务器无法访问 GHCR，或需要测试未发布代码时才使用源码构建：

```bash
docker compose up -d --build
```

这会在 Docker builder 中用 pnpm 11 构建 Web，并编译 Rust Release 二进制和固定版本的 libvips，首次构建会明显慢于拉取镜像。仅拉取已发布镜像时不要使用 `--build`。

### 访问 Web 控制台

Web 控制台已经位于镜像的 `/app/web`，Axum 会提供首页、认证页面、`/app/*`、`/admin/*` 和构建后的静态资源。生产环境通常只需要一个公网 Origin：

```dotenv
MEDIAHUB_WEB_URL=https://mediahub.example.com
MEDIAHUB_CORS_ALLOWED_ORIGINS=
```

`MEDIAHUB_WEB_URL` 用于生成验证邮箱和重置密码链接，必须与用户实际访问的 HTTPS Origin 一致。同源模式不需要 CORS 白名单，生产 Web 会自动调用当前 Origin 的 API。

本地开发时，后端和 Vite 可以分别启动；Vite 开发模式默认调用当前主机的 `3000` 端口：

```bash
cd web
pnpm install --frozen-lockfile
pnpm dev
```

### 配置

从 `.env.example` 开始配置。`.env` 只放在服务器或 Secret 管理系统中，不要提交到 Git。完整配置、备份恢复和测试命令见 [`docs/runbook.md`](docs/runbook.md)。

| 配置 | 作用 |
| --- | --- |
| `MEDIAHUB_IMAGE` | Web/API/Worker 镜像地址，生产环境建议固定摘要 |
| `MEDIAHUB_POSTGRES_DB`、`MEDIAHUB_POSTGRES_USER`、`MEDIAHUB_POSTGRES_PASSWORD` | Compose PostgreSQL 配置；共享环境必须替换默认密码 |
| `MEDIAHUB_DATABASE_URL` | 可选，覆盖 Compose 自动生成的连接串，用于外部 PostgreSQL |
| `MEDIAHUB_STORAGE_BACKEND` | `local` 或 `s3`，默认是 `local` |
| `MEDIAHUB_STORAGE_ROOT` | Local 模式的对象目录，Compose 中为 `/data/storage` |
| `MEDIAHUB_S3_*` | S3 兼容后端的 Bucket、Region、Endpoint、凭证、前缀和寻址方式 |
| `MEDIAHUB_ACCESS_KEY_MASTER_KEY` | 独立的 Base64 32 字节 AES-256-GCM 主密钥，用于加密 Access Key 和 Webhook Secret |
| `MEDIAHUB_ACCESS_KEY_MASTER_KEY_VERSION`、`MEDIAHUB_ACCESS_KEY_MASTER_KEYRING` | 主密钥轮换时的版本和旧密钥环，旧密钥不能立即删除 |
| `MEDIAHUB_MEDIA_SIGNING_KEY` | 另一个独立的 Base64 密钥，用于签发短期媒体和上传链接，不能与主密钥复用 |
| `MEDIAHUB_RESEND_API_KEY` | Resend 服务端 API Key，只放在 API 容器，不要暴露给 Web |
| `MEDIAHUB_EMAIL_FROM` | Resend 中已验证域名的发件人地址 |
| `MEDIAHUB_WEB_URL` | Web 控制台的纯 HTTPS Origin，用于验证和重置链接 |
| `MEDIAHUB_REGISTRATION_ENABLED` | 是否开放注册；完成首个管理员初始化后建议设为 `false` |
| `MEDIAHUB_CORS_ALLOWED_ORIGINS` | 可选跨 Origin 客户端的精确逗号分隔白名单；内置同源 Web 保持为空 |
| `MEDIAHUB_COOKIE_SAME_SITE`、`MEDIAHUB_ALLOW_INSECURE_COOKIES` | Cookie 跨站策略；生产 HTTPS 不要启用不安全 Cookie |
| `MEDIAHUB_BOOTSTRAP_ADMIN_EMAIL` | 一次性将已验证用户提升为系统管理员，成功后必须删除 |
| `MEDIAHUB_METRICS_BEARER_TOKEN` | 可选的 Prometheus `/metrics` Bearer Token |

三个关键密钥必须分别生成并持久保存：数据库密码用于登录 PostgreSQL，Access Key 主密钥用于解密存储的凭证，媒体签名密钥用于验证短期 URL。重新部署镜像时必须继续使用原值；丢失 Access Key 主密钥会导致已加密的 Secret 无法恢复。

Resend 需要先验证发件域名。MediaHub 直接调用 `https://api.resend.com/emails` 发送邮箱验证和密码重置邮件；如果没有配置 Resend，只能在隔离开发环境启用 `MEDIAHUB_EXPOSE_AUTH_TOKENS=true`，不能用于公网部署。

### 首次管理员初始化

1. 临时设置 `MEDIAHUB_REGISTRATION_ENABLED=true`，启动 API。
2. 从 Web 控制台注册账号，并通过 Resend 邮件完成验证。
3. 设置 `MEDIAHUB_BOOTSTRAP_ADMIN_EMAIL` 为该账号邮箱，执行 `docker compose up -d --no-build` 一次。
4. 确认日志显示管理员提升成功后，立即删除 `MEDIAHUB_BOOTSTRAP_ADMIN_EMAIL`，并将 `MEDIAHUB_REGISTRATION_ENABLED=false`。

重复保留 Bootstrap 变量会让后续启动主动失败，这是设计上的 fail-closed 保护。

## 支持的协议与入口

### JSON 控制面 API

`/api/v1/*` 是 Web 控制台和业务后端使用的 JSON API，包含认证、Application、Access Key、Bucket、媒体、上传、Webhook、AsyncJob、审计和管理员接口。完整请求/响应契约见 [`openapi/openapi.json`](openapi/openapi.json)。

浏览器通常使用 Session Cookie；程序化客户端使用 Application 级 Access Key 和 HMAC-SHA256 签名。写操作需要新鲜的 `X-MediaHub-Nonce`，创建 Bucket 和 UploadSession 还需要 `Idempotency-Key`。

### 原生路径对象 API

路径入口用于直接操作 Application、Bucket 和对象：

```text
GET    /{app_id}
GET    /{app_id}/{bucket}
PUT    /{app_id}/{bucket}/{object_key}
GET    /{app_id}/{bucket}/{object_key}
HEAD   /{app_id}/{bucket}/{object_key}
PATCH  /{app_id}/{bucket}/{object_key}
POST   /{app_id}/{bucket}/{object_key}
DELETE /{app_id}/{bucket}/{object_key}
```

对象内容不可覆盖；重复写入同一 Object Key 会返回冲突。可见性、Metadata、生命周期、预签名访问和异步删除仍由 MediaHub 的 Application/Bucket 策略控制。

### WebDAV

WebDAV 挂载在 `/dav`：

```text
/dav/{app_id}/{bucket}/...
```

使用 Application Access Key ID 作为 Basic Auth 用户名，创建时返回的一次性 Secret 作为密码。支持 `PROPFIND`、`GET`、`HEAD`、Range、受限 `PUT`、`MKCOL`、`COPY`、`MOVE`、`DELETE` 和锁发现。WebDAV 使用与 JSON API 相同的 PostgreSQL、配额、Bucket 策略、Local/S3 存储、审计和异步删除流程，不会直接暴露本地文件目录。

### S3 兼容存储后端

将 `MEDIAHUB_STORAGE_BACKEND=s3` 后，MediaHub 使用外部 S3 或 S3 兼容服务保存对象二进制，PostgreSQL 仍保存 Metadata、权限、配额、Variant 和任务状态。需要配置：

```dotenv
MEDIAHUB_STORAGE_BACKEND=s3
MEDIAHUB_S3_BUCKET=mediahub-production
MEDIAHUB_S3_REGION=us-east-1
MEDIAHUB_S3_ENDPOINT=https://s3.example.com
MEDIAHUB_S3_ACCESS_KEY_ID=...
MEDIAHUB_S3_SECRET_ACCESS_KEY=...
MEDIAHUB_S3_PREFIX=mediahub
MEDIAHUB_S3_VIRTUAL_HOSTED_STYLE=false
MEDIAHUB_S3_ALLOW_HTTP=false
```

生产 S3 Endpoint 必须使用 HTTPS。AWS S3 可省略 `MEDIAHUB_S3_ENDPOINT`；只有要求 Bucket 出现在域名中的服务才启用 `MEDIAHUB_S3_VIRTUAL_HOSTED_STYLE=true`。Local 模式则使用 Compose 的 `mediahub-data` Volume。

### 受限 S3 网关

`/s3` 是给受控 SDK 集成使用的路径式网关，不是完整的通用 S3 服务：

```text
/s3/{bucket}
/s3/{bucket}/{object_key}
```

当前支持对象的 `PutObject`、`GetObject`、`HeadObject`、删除和受限列表，单次 PUT 上限为 64 MiB。客户端使用 Application Access Key，要求至少具备 `media:upload` 和 `media:read`，并使用 `force_path_style=true`。是否启用网关可以通过 `GET /api/v1/capabilities` 的 `s3_gateway` 字段发现。它不能替代完整的 AWS S3 管理 API、Multipart Upload 或 Bucket 管理服务。

### 健康检查和运行边界

```text
GET /health/live
GET /health/ready
GET /api/v1/capabilities
GET /metrics
```

`/metrics` 不是匿名接口，需要管理员 Session 或 `MEDIAHUB_METRICS_BEARER_TOKEN`。生产环境应在 Nginx、Caddy、Cloudflare Tunnel 等反向代理处终止 TLS，不要将 PostgreSQL 端口暴露到公网。Compose 默认只把 PostgreSQL 绑定到 `127.0.0.1`。

PostgreSQL 元数据、Local 对象、密钥环和部署 Secret 必须作为一个整体备份。详细的 WAL/PITR、对象快照、S3 配置、HMAC 和测试命令见 [`docs/runbook.md`](docs/runbook.md)。

本项目采用 MIT 许可证。安全问题请按 [`SECURITY.md`](SECURITY.md) 私下报告，贡献流程见 [`CONTRIBUTING.md`](CONTRIBUTING.md)。

## 1. 项目定位

MediaHub 是一个面向图片、视频和其他静态媒体的资源中心。它提供类似 OSS、COS 的对象存储能力，同时内置图片处理、生命周期和可扩展 Metadata，适合多个 AI 网站或内部服务统一接入。

```text
Browser / AI Service / Business Backend
                  |
                  v
              MediaHub API
          /          |          \\
   Metadata DB   Object Storage   Async Worker
                                  |
                                  v
                              CDN / Webhook
```

MediaHub 负责：

- 用户注册、登录与控制台会话
- Application、AccessKey 和 Bucket 管理
- 图片、视频及静态文件的上传、读取和删除
- 公开、私有及预签名访问
- 图片实时转换与派生缓存
- 对象 Metadata，包括 AI 产物的扩展信息
- TTL、保留数量、归档和异步删除
- 配额、审计、Webhook、后台管理
- 本地存储，以及兼容 S3 的存储后端

MediaHub 不负责：

- AI 生成任务调度、队列、计费或失败重试
- Prompt 工作流、模型调用和生成历史业务逻辑
- 将外部业务系统的任务关系建模到核心数据库
- 首个版本的视频转码平台、内容审核平台或通用网盘协作

AI 模型、Prompt、Seed 等信息只是对象 Metadata。MediaHub 负责安全保存和返回这些信息，但不解释其业务含义。

## 2. 核心原则

### 2.1 文件内容不可变

对象上传完成后，二进制内容永远不覆盖。需要修改内容时必须创建新对象和新的对象 ID。

以下属性可以修改：

- 用户 Metadata
- AI Metadata
- 可见性
- 生命周期策略或过期时间
- 对象显示名称

该原则让内容哈希、ETag、CDN 缓存和派生文件始终可预测。

### 2.2 Metadata 与二进制分离

数据库只保存对象属性、权限、状态、哈希和存储位置，不保存图片或视频 Binary。二进制写入 Storage Backend。

### 2.3 身份与资源归属分离

```text
User 1 --- N Application 1 --- N Bucket 1 --- N Media
                         |
                         +--- N AccessKey
```

- User：登录 Web UI 的人。
- Application：资源和配额的归属主体，拥有公开的 AppId。
- AccessKey：程序访问 API 的凭证。
- Bucket：对象命名、权限和生命周期边界。
- Media：一个不可变的媒体对象。

用户注册后自动创建一个默认 Application。一个用户以后可以管理多个 Application，而不需要迁移已有对象。

### 2.4 默认安全

- API 不接受调用方通过 `user_id` 或邮箱指定资源所有者。
- 身份只能由登录 Session 或 AccessKey 推导。
- SecretAccessKey 不进入浏览器、不写入日志、创建后只展示一次。
- 私有 Bucket 默认不允许匿名读取。
- 上传内容必须经过大小、类型和媒体结构校验。

### 2.5 异步操作必须幂等

删除、归档、Webhook、缓存清理和存储同步均可能失败或重复执行。所有后台任务必须支持安全重试，并最终收敛到一致状态。

### 2.6 核心逻辑与运行时分离

V1 只交付 Docker 部署，但架构必须允许未来增加 Cloudflare Native Profile。领域模型、权限、生命周期、配额、签名规范和状态机不能依赖 Axum、Tokio、SQLx、本地文件系统或 libvips。

运行时能力通过明确的 Port 接入，Docker 和 Cloudflare 分别提供 Adapter。两个部署 Profile 共享 API 契约、领域规则和一致性测试，但不要求共享同一个可执行文件，也不承诺所有媒体处理能力完全相同。

## 3. 核心术语

| 术语            | 含义                               |
| --------------- | ---------------------------------- |
| User            | 使用邮箱和密码登录控制台的人类用户 |
| Application     | 一个网站、服务或项目的资源空间     |
| AppId           | Application 的公开标识，不是秘密   |
| AccessKeyId     | API 凭证的公开标识，用于定位密钥   |
| SecretAccessKey | 计算 HMAC 签名的秘密，只展示一次   |
| Bucket          | 对象的逻辑容器和策略边界           |
| Media           | 一份不可变的原始二进制对象         |
| Variant         | 由原始对象转换产生的可重建派生文件 |
| Object Key      | 对象在 Bucket 内的唯一名称         |
| Metadata        | 与对象关联的结构化扩展信息         |
| Storage Key     | MediaHub 内部使用的物理存储路径    |

## 4. 总体架构

MediaHub 采用 Core + Port + Adapter 结构。V1 的 Docker Profile 可以在同一个 Rust 进程内运行 API 和 Worker；未来 Cloudflare Native Profile 使用独立运行时入口复用 Core 和 API 契约。

```text
                    +-----------------------+
                    |   Bundled Vite Web    |
                    +-----------+-----------+
                                |
                         OpenAPI / HTTP
                                |
              +-----------------+-----------------+
              |                                   |
     +--------v---------+                +--------v---------+
     | Docker Runtime   |                | CF Runtime       |
     | Axum + Tokio     |                | Workers (future) |
     +--------+---------+                +--------+---------+
              |                                   |
              +-----------------+-----------------+
                                |
                     +----------v-----------+
                     | Application + Core   |
                     | Rules / State Machine|
                     +----------+-----------+
                                |
          +---------------------+----------------------+
          | Repository | ObjectStore | Processor | Jobs |
          +---------------------+----------------------+
```

组件职责：

- Runtime Entry：接收 HTTP、解析运行时绑定并调用 Application Use Case。
- Application/Core：认证后的业务规则、权限、状态机、配额和生命周期，不感知部署平台。
- API Contract：统一路由、DTO、错误码、签名规范和 OpenAPI。
- Repository：User、Application、Bucket、Media、任务和事件持久化。
- ObjectStore：原始对象和 Variant 二进制存储。
- ImageProcessor：图片转换能力。
- JobDispatcher：删除、生命周期、Webhook 和 Reconciler 等异步任务。
- Media Handler：Range、ETag、图片转换、缓存响应。
- CDN：公开对象和可缓存 Variant 的边缘分发。

建议的代码结构：

```text
mediaHub/
├── crates/
│   ├── mediahub-core/          领域实体、规则、状态机
│   ├── mediahub-app/           Use Case 与 Port
│   ├── mediahub-server/        Axum/Tokio Docker 入口
│   ├── adapter-sqlx/
│   ├── adapter-local/
│   ├── adapter-s3/
│   └── adapter-libvips/
├── apps/
│   └── cloudflare-worker/      未来 Cloudflare Runtime 入口
├── migrations/
├── openapi/
└── web/                        Vite + React Web UI
```

`mediahub-core` 和 `mediahub-app` 不得引用 Axum Request、SQLx Row、文件路径或 Cloudflare Binding 类型。运行时 DTO 在入口层转换为 Core 类型。

## 5. 本地存储布局

```text
storage/
├── objects/
│   └── 2026/
│       └── 07/
│           ├── <media_id>.png
│           └── <media_id>.mp4
├── cache/
│   ├── image/
│   └── video/
├── temp/
├── trash/
└── quarantine/
```

- 物理路径使用 MediaHub 生成的 ID，不直接使用用户提交的 Object Key。
- `temp` 保存尚未提交的上传，可由后台按时间清理。
- `trash` 保存等待物理删除的本地对象，可配置短暂恢复期。
- `quarantine` 保存校验失败或安全扫描命中的文件，默认不能读取。
- Variant 属于可重建缓存，可以按容量或最近访问时间淘汰。

## 6. 数据模型

以下字段用于定义实体和约束，迁移脚本使用 PostgreSQL 实现。所有时间统一保存为 UTC，API 使用 RFC 3339。

### 6.1 User

```sql
id
email
email_normalized
password_hash
email_verified_at
status
system_role
last_login_at
created_at
updated_at
```

约束：

- `email_normalized` 全局唯一。
- `password_hash` 使用 Argon2id。
- `status`：`pending_verification`、`active`、`suspended`、`deleted`。
- `system_role`：`user` 或 `admin`，默认 `user`；普通用户 API 不允许修改该字段。
- 删除用户必须通过异步流程处理其 Application，不能直接级联物理删除所有对象。

### 6.2 Session

```sql
id
user_id
token_hash
expires_at
last_seen_at
created_ip
last_seen_ip
user_agent_summary
revoked_at
created_at
```

浏览器只持有随机 Session Token，数据库保存 Token 哈希。Cookie 必须启用 `HttpOnly`、`Secure` 和合适的 `SameSite` 策略。

邮箱验证和密码重置使用独立的一次性 Token 表，数据库同样只保存 Token 哈希，并设置较短过期时间。

### 6.3 OneTimeToken

```sql
id
user_id
purpose
token_hash
expires_at
consumed_at
created_at
```

- `purpose`：`verify_email` 或 `reset_password`。
- 数据库只保存 Token Hash，原始 Token 只发送给用户。
- Token 必须短时有效、一次性使用，并限制签发和验证频率。
- 消费时使用 `consumed_at IS NULL AND expires_at > now()` 条件更新，确保并发请求中只有一次成功。
- 密码重置成功后撤销该 User 的全部 Session；无论邮箱是否存在，申请接口返回相同响应。

### 6.4 Application

```sql
id
app_id
owner_user_id
name
status
quota_bytes
used_bytes
reserved_bytes
created_at
updated_at
```

- `app_id` 全局唯一，例如 `app_01J...`。
- `status`：`active`、`suspended`、`deleting`、`deleted`。
- `used_bytes` 是已提交对象占用量。
- `reserved_bytes` 是进行中上传预占的配额。

V1 由单个 User 拥有 Application。未来需要团队协作时，可以增加 Organization 和 Membership，不改变 Bucket、Media 的归属关系。

### 6.5 AccessKey

```sql
id
application_id
access_key_id
secret_ciphertext
secret_key_version
secret_last_four
name
permissions_json
expires_at
last_used_at
last_used_ip
revoked_at
created_at
```

- `access_key_id` 全局唯一，例如 `mh_ak_01J...`。
- SecretAccessKey 由密码学安全随机数生成器生成。
- Secret 使用部署主密钥加密保存，以支持 HMAC 验签；不能只保存普通密码哈希。
- 主密钥不得存入数据库，必须通过 Secret Manager、Docker Secret 或受保护环境变量提供。
- Secret 创建后只返回一次，后续只能轮换或重新创建。
- 一个 Application 可创建多个独立命名、独立权限和独立过期时间的密钥。

### 6.6 Bucket

```sql
id
application_id
name
visibility
default_ttl_seconds
max_object_size
allowed_mime_types_json
lifecycle_policy_json
created_at
updated_at
```

约束：

```sql
UNIQUE(application_id, name)
```

- Bucket 名称创建后不可修改。
- `visibility`：`public` 或 `private`。
- `default_ttl_seconds = NULL` 表示默认永不过期。
- Bucket 删除必须为空，或显式提交异步强制删除任务。

### 6.7 Media

```sql
id
application_id
bucket_id
object_key
original_name
display_name
mime
extension
size
width
height
duration_ms
sha256
etag
storage_backend
storage_key
status
visibility_override
metadata_json
metadata_version
revision
expire_at
archived_at
deleted_at
created_at
updated_at
```

核心约束和索引：

```sql
UNIQUE(bucket_id, object_key)
INDEX(application_id, created_at)
INDEX(bucket_id, created_at)
INDEX(status, expire_at)
INDEX(sha256)
```

- `id` 建议使用 UUIDv7 或其他按时间大致有序且不可预测的 ID。
- Object Key 在 Bucket 内唯一，禁止覆盖；冲突返回 `409 Conflict`。
- `storage_key` 只能由服务端生成，不能直接使用用户输入，防止路径穿越。
- `etag` 对原始对象可使用内容 SHA-256 的稳定编码。
- `visibility_override` 为空时继承 Bucket 设置。
- `updated_at` 只反映 Metadata、权限或生命周期变化，不表示文件内容被修改。
- `revision` 每次可变属性更新时递增，用于避免并发修改互相覆盖。

对象状态：

```text
uploading
  | success
  v
active ---> archive_pending ---> archived
  |
  +-------> delete_pending -----> deleted
  |
  +-------> quarantined
```

只有 `active` 对象能够正常读取。`archived` 对象需要先完成恢复，恢复期间返回明确的不可用状态。

### 6.8 Variant

```sql
id
media_id
transform_key
parameters_json
processor_version
format
width
height
size
storage_backend
storage_key
status
last_accessed_at
created_at
```

约束：

```sql
UNIQUE(media_id, transform_key)
```

Variant 是可重建资源。删除原始 Media 时必须使其所有 Variant 失效并进入清理队列。

### 6.9 WebhookEndpoint 与 OutboxEvent

```sql
WebhookEndpoint
---------------
id
application_id
url
secret_ciphertext
event_types_json
status
created_at

OutboxEvent
-----------
id
application_id
event_type
aggregate_id
payload_json
status
attempt_count
next_attempt_at
created_at
delivered_at
```

业务状态变化和 OutboxEvent 必须在同一个数据库事务中提交，避免对象已创建但 Webhook 永久丢失。

### 6.10 AsyncJob 与 IdempotencyKey

异步删除、归档、恢复、缓存清理和存储迁移使用通用 AsyncJob，不包含 AI 生成业务。

上传和批量修改接口支持 `Idempotency-Key`：

```sql
IdempotencyKey
--------------
id
application_id
operation_scope
idempotency_key
request_hash
status
response_status
response_payload
resource_id
expires_at
created_at
completed_at
```

约束：

```sql
UNIQUE(application_id, operation_scope, idempotency_key)
```

- 相同 Key 和相同请求哈希返回第一次已完成结果，处理中返回稳定的处理中状态。
- 相同 Key 但请求哈希不同返回 `409 Conflict`。
- 记录必须有明确保留期，过期清理由后台任务执行。
- 失败是否允许用同一个 Key 重试必须由错误类型决定，不能静默创建第二个资源。
- `response_payload` 不得明文保存 Secret、Session 或签名 URL；敏感创建接口使用短期加密响应或只保存资源引用，并在 API 文档中明确重试语义。

HMAC Mutation 请求使用短期 ReplayNonce：

```sql
ReplayNonce
-----------
access_key_id
nonce
expires_at
created_at
```

约束：

```sql
UNIQUE(access_key_id, nonce)
```

所有部署 Profile 都将 Nonce 持久化到 PostgreSQL；边缘运行时可以使用 Durable Object 辅助限流，但不能替代 PostgreSQL 中的防重放记录。Nonce 只用于短期防重放，不能替代 IdempotencyKey 的结果恢复语义。

### 6.11 UploadSession

```sql
id
application_id
bucket_id
object_key
storage_backend
storage_key
multipart_upload_id
status
expected_size
actual_size
checksum_algorithm
expected_checksum
verified_checksum
reserved_bytes
expires_at
completed_at
created_at
updated_at
```

状态：

```text
created -> uploading -> completing -> completed
                    \\-> aborted
                    \\-> expired
```

- 创建 Session 时预占 Object Key 和配额，避免并发上传同名对象。
- 单个对象的技术上限为 2 GiB，Application 配额和 Bucket `max_object_size` 仍可设置更小的业务限制。Local 网关必须将对象流式写入磁盘，不能把通用 API 请求体上限复用为对象上限，也不能将完整对象聚合进内存。
- `storage_key` 在上传开始前由服务端生成，最终对象直接写入该 Key；对象是否可见由 Media/UploadSession 状态控制。
- `multipart_upload_id` 只在后端需要时保存，不暴露为稳定业务标识。
- 默认使用 SHA-256；Adapter 不能把 Multipart ETag 当作内容哈希。
- 完成操作必须幂等，同一 Session 只能生成一个 Media。
- 取消、超时或校验失败必须终止 Multipart、删除未提交对象并释放预占配额。
- Worker 定期回收过期 Session，Reconciler 对账 Session、Media 和 ObjectStore。

### 6.12 AuditLog

```sql
id
application_id
actor_type
actor_id
action
target_type
target_id
request_id
ip
summary_json
created_at
```

AuditLog 只记录操作摘要，不保存密码、Secret、Session、完整 Prompt 或文件内容。审计记录只能追加，保留周期由部署配置决定。

## 7. 对象 Metadata

Metadata 是 MediaHub 适应 AI 时代产物的核心扩展点，但它保持为对象属性，不形成 AI 任务模型。

推荐 API 表现形式：

```json
{
  \"system\": {
    \"mime\": \"image/png\",
    \"size\": 2459123,
    \"width\": 1024,
    \"height\": 1024,
    \"sha256\": \"...\"
  },
  \"user\": {
    \"project\": \"website-a\",
    \"external_generation_id\": \"gen_123\",
    \"output_index\": 0
  },
  \"ai\": {
    \"provider\": \"openai\",
    \"model\": \"gpt-image-2\",
    \"model_revision\": null,
    \"prompt\": \"...\",
    \"negative_prompt\": null,
    \"seed\": null,
    \"steps\": null,
    \"sampler\": null,
    \"cfg\": null
  }
}
```

规则：

- `system` 由 MediaHub 生成，只读；上传时提交该命名空间应被拒绝或忽略。
- `user` 由调用方自由扩展。
- `ai` 是推荐约定，不要求每个模型提供所有字段。
- `metadata_version` 用于未来结构迁移。
- `revision` 用于对象可变属性的乐观并发控制，与 Metadata Schema 版本不是同一概念。
- 默认限制序列化后 Metadata 不超过 64 KiB，并限制嵌套深度、键数量和字符串长度。
- Prompt 和其他 Metadata 不写入公开文件响应头，不随公开 CDN URL 暴露。
- Metadata 只能通过已认证并授权的管理 API 读取。
- V1 只支持整对象读取和更新。需要检索时，再为少量常用键增加受控索引，避免绑定任意 JSON 查询语法。

文件删除时 Metadata 随 Media 进入相同的保留和清除流程。

## 8. 认证与授权

### 8.1 邮箱注册登录

V1 支持邮箱和密码：

```text
POST /api/v1/auth/register
POST /api/v1/auth/verify-email
POST /api/v1/auth/login
POST /api/v1/auth/logout
POST /api/v1/auth/forgot-password
POST /api/v1/auth/reset-password
GET  /api/v1/auth/me
```

安全要求：

- 密码使用 Argon2id，并允许未来提升参数后重新哈希。
- 注册必须验证邮箱；邮件发送通过 Resend API 完成。
- 登录、注册、验证码和密码重置均需限流。
- 登录失败统一返回模糊错误，避免枚举已注册邮箱。
- 重置 Token 一次性使用且短时有效。
- Self-hosted 部署可关闭公开注册，仅允许管理员邀请。

### 8.2 Application 和密钥管理

```text
GET    /api/v1/applications
POST   /api/v1/applications
GET    /api/v1/applications/{app_id}
PATCH  /api/v1/applications/{app_id}
DELETE /api/v1/applications/{app_id}

GET    /api/v1/applications/{app_id}/access-keys
POST   /api/v1/applications/{app_id}/access-keys
PATCH  /api/v1/access-keys/{access_key_id}
DELETE /api/v1/access-keys/{access_key_id}
```

密钥创建响应示例：

```json
{
  \"app_id\": \"app_01J...\",
  \"access_key_id\": \"mh_ak_01J...\",
  \"secret_access_key\": \"仅本次展示\",
  \"expires_at\": null
}
```

### 8.3 HMAC API 签名

业务服务使用 AccessKeyId 和 SecretAccessKey 对请求签名。Canonical Request 至少覆盖：

```text
HTTP Method
Canonical Path
排序并编码后的 Query
参与签名的 Headers
Body SHA-256
Timestamp
Nonce（Mutation 请求）
Idempotency-Key（需要幂等结果的 Mutation 请求）
```

建议请求头：

```text
X-MediaHub-Access-Key: mh_ak_01J...
X-MediaHub-Date: 20260714T080000Z
X-MediaHub-Content-SHA256: ...
X-MediaHub-Nonce: ...
Idempotency-Key: ...
Authorization: MH-HMAC-SHA256 SignedHeaders=...; Signature=...
```

- 服务端默认只接受时间偏差不超过 5 分钟的请求。
- 签名比较使用常量时间算法。
- 所有创建、修改和删除请求必须携带密码学安全随机 Nonce，Nonce 必须参与签名且在有效窗口内只能成功使用一次。
- 创建资源、批量操作等需要安全重试的请求还必须携带 Idempotency-Key；该字段和请求体哈希必须参与签名。
- 每次网络重试使用新的 Nonce，但保持相同 Idempotency-Key 和请求内容。
- 已使用 Nonce 的重放请求即使签名和时间仍有效也返回 `401` 或专用重放错误；相同 Idempotency-Key 的合法重试通过幂等记录恢复结果。
- Query 和 Header 的规范化规则必须形成单独协议文档和测试向量。
- 大文件流式上传可以使用已签名的预期内容哈希，或专门的流式签名协议；V1 优先使用预签名上传 URL。
- 预签名下载 URL 不使用 Nonce，但必须只授权固定 Method、对象、转换参数和短时过期时间；签发后默认无法单独撤销，紧急撤销依赖对象状态、密钥轮换或 CDN Purge。

### 8.4 权限范围

AccessKey 权限至少支持：

```text
application:read
bucket:list
bucket:manage
media:list
media:read
media:upload
media:update
media:delete
webhook:manage
```

权限还可以限制到指定 Bucket 和 Object Key Prefix。权限判断必须同时验证 Application 归属，不能只检查对象 ID。

### 8.5 浏览器直传

SecretAccessKey 绝不能发送到浏览器。推荐流程：

```text
Browser
  -> Business Backend：申请上传
  -> MediaHub：后端使用 AccessKey 请求预签名 URL
  <- Business Backend：返回短时上传 URL
  -> MediaHub：Browser 直接上传文件
```

预签名 URL 必须绑定 HTTP Method、Bucket、Object Key、最大大小、内容类型和过期时间，默认有效期不超过 15 分钟。

### 8.6 系统管理员

系统管理员和普通用户控制台使用同一 User 身份体系，但授权边界独立：

```text
GET   /api/v1/admin/users
PATCH /api/v1/admin/users/{id}/status
GET   /api/v1/admin/applications
GET   /api/v1/admin/jobs
GET   /api/v1/admin/storage
GET   /api/v1/admin/audit
```

- 所有 `/api/v1/admin/*` 路由要求 `system_role = admin`，不能只依赖前端隐藏入口。
- 普通 User API 永远不能修改 `system_role`。
- 首个管理员通过一次性 CLI 或部署环境变量引导创建；引导完成后必须禁用 Bootstrap 凭证。
- 管理员封禁 User/Application 使用审计日志和显式状态迁移，不直接删除数据。
- 系统后台显示全局存储、任务和用户；普通控制台只显示当前 User 拥有的 Application。
- 生产环境建议要求管理员启用 MFA；MFA 可以后置到 V1.1，但数据模型和路由不得阻止增加第二认证因子。

### 8.7 Session 生命周期

```text
GET    /api/v1/auth/sessions
DELETE /api/v1/auth/sessions/{id}
DELETE /api/v1/auth/sessions
```

- 登录、权限提升和密码重置后必须旋转 Session，防止 Session Fixation。
- 密码重置、用户封禁和账户删除撤销全部已有 Session。
- 用户可以查看登录时间、最近活动、设备摘要并退出其他设备。
- 每个用户限制活跃 Session 数，超出后撤销最旧 Session。
- Session Token 只通过 Host-only Cookie 发送；明确设置 Path、Max-Age、HttpOnly、Secure 和 SameSite。
- Pages 跨 Origin 调用 API 时必须使用严格 Origin 校验、CSRF Token 和凭证 CORS，不能只依赖 SameSite。

### 8.8 主密钥 Keyring

AccessKey Secret、Webhook Secret 等可恢复凭证使用版本化 Keyring 加密：

```text
MASTER_KEY_V1
MASTER_KEY_V2
ACTIVE_MASTER_KEY_VERSION=2
```

- 新写入使用 Active Version，每条密文保存 `secret_key_version`。
- 轮换时先增加新 Key，再切换 Active Version，最后由后台幂等重加密旧记录。
- 所有旧记录完成重加密且备份过期后才能移除旧 Key。
- 数据库恢复必须同时获得所需 Keyring；缺少仍被引用的 Key 时服务必须 Fail Closed，并在 Readiness 中报告不可用。
- Keyring 独立于数据库和普通配置备份，恢复演练必须验证 AccessKey 验签和 Webhook 签名。
- Key Material 不写入日志、数据库、镜像层或前端构建产物。

## 9. Bucket

```text
GET    /api/v1/buckets
POST   /api/v1/buckets
GET    /api/v1/buckets/{name}
PATCH  /api/v1/buckets/{name}
DELETE /api/v1/buckets/{name}
```

创建示例：

```json
{
  \"name\": \"outputs\",
  \"visibility\": \"private\",
  \"default_ttl_seconds\": 2592000,
  \"max_object_size\": 20971520,
  \"allowed_mime_types\": [\"image/png\", \"image/jpeg\", \"image/webp\"]
}
```

建议的默认 Bucket 由用户自行创建，不在核心代码中硬编码 `avatars`、`images`、`outputs` 等业务名称。

## 10. 对象 API

### 10.1 上传

```text
POST /api/v1/media
Content-Type: multipart/form-data
```

字段：

```text
file        必填，二进制文件
bucket      必填，Bucket 名称
object_key  可选，默认由 MediaHub 生成
ttl_seconds 可选，正整数秒覆盖 Bucket 默认值；null 表示永不过期
visibility  可选，覆盖 Bucket 默认值
metadata    可选，JSON 字符串，只允许 user/ai 命名空间
```

请求不接受 `user` 或 `application_id` 来决定归属，Application 必须从认证上下文获得。

省略 `object_key` 时，服务端生成：

```text
auto/{UTC-yyyy}/{UTC-MM}/{uuidv7}
```

自动 Key 不带扩展名。真实格式保存在 `mime` 和 `extension`，原始名称保存在 `original_name`。用户指定的 Object Key 仍需经过长度、编码和路径规范化校验。

成功响应：

```json
{
  \"id\": \"019f...\",
  \"app_id\": \"app_01J...\",
  \"bucket\": \"outputs\",
  \"object_key\": \"auto/2026/07/019f...\",
  \"mime\": \"image/png\",
  \"size\": 2459123,
  \"etag\": \"sha256:...\",
  \"status\": \"active\",
  \"visibility\": \"private\",
  \"expire_at\": \"2026-08-13T08:00:00Z\",
  \"content_url\": null
}
```

`content_url` 只为公开对象返回稳定 CDN URL。私有对象返回 `null`，调用方通过以下接口获取短时签名 URL：

```text
POST /api/v1/media/{id}/signed-url
```

### 10.2 Docker Local 原子上传流程

以下流程是 Local Adapter 的实现。S3/R2 直传使用 Upload Session，不依赖文件 rename：

```text
1. 认证和授权
2. 校验 Bucket 策略并预占配额
3. 创建 uploading 记录或上传预留记录
4. 流式写入 temp，同时计算大小和 SHA-256
5. 检测真实 MIME，解析必要媒体属性
6. 校验大小、类型、图片像素等限制
7. fsync 并原子移动到最终 Storage Key
8. 数据库事务：Media -> active、提交配额、写入 OutboxEvent(media.uploaded)
9. 异步投递 media.uploaded Webhook
```

任一步失败必须释放预占配额并清理临时文件。进程在步骤 7 与 8 之间崩溃可能产生孤儿文件，后台 Reconciler 必须识别并清理。

### 10.2.1 Upload Session 与直传

浏览器和大文件上传使用后端无关的 Upload Session：

```text
POST   /api/v1/uploads
POST   /api/v1/uploads/{upload_id}/complete
DELETE /api/v1/uploads/{upload_id}
```

创建请求包含 Bucket、Object Key、预期大小、内容类型和 Metadata。响应示例：

```json
{
  \"upload_id\": \"upl_01J...\",
  \"method\": \"PUT\",
  \"url\": \"https://upload-target.example/...\",
  \"headers\": {
    \"content-type\": \"image/png\"
  },
  \"expires_at\": \"2026-07-14T08:15:00Z\"
}
```

URL 可能指向 MediaHub、S3 Compatible Storage 或 R2，调用方不能依赖 URL 结构判断部署类型。完成接口通过 ObjectStore `head` 校验大小、媒体属性和独立 Checksum 后，才提交 Media、配额和 OutboxEvent。ETag 只用于对象版本识别，除非 Adapter 明确证明其算法，否则不能作为 SHA-256。取消或超时的 Upload Session 必须终止 Multipart、释放配额预占并清理未提交对象。

Adapter 必须声明是否能在上传前硬性约束总大小和 Checksum。无法硬限制 Presigned PUT 大小时，完成接口仍要删除超限对象并拒绝提交，但这不能阻止临时存储和流量成本；要求严格上传前限制的 Bucket 必须经 MediaHub/Worker 流式网关，或使用可约束 Part 与总大小的 Multipart 协议。

### 10.3 查询和更新 Metadata

```text
GET   /api/v1/media/{id}
PATCH /api/v1/media/{id}
```

PATCH 只允许修改：

- `display_name`
- `visibility`
- `ttl_seconds` 或 `expire_at`
- `metadata.user`
- `metadata.ai`

不允许修改对象内容、Bucket、Object Key、哈希和系统 Metadata。

PATCH 应携带当前对象 Revision，例如 `If-Match: \"meta-7\"`。版本不一致时返回 `409 Conflict`，调用方重新读取后再决定是否覆盖。

### 10.4 列表

```text
GET /api/v1/media?bucket=outputs&status=active&limit=50&cursor=...
GET /api/v1/media?bucket=outputs&prefix=images/&delimiter=/&limit=50&cursor=...
```

- 使用稳定的 Cursor Pagination，不使用大偏移量分页。
- 默认按 `created_at DESC, id DESC` 排序。
- 支持按 Bucket、状态、MIME、创建时间和 Object Key Prefix 过滤。
- 指定 Bucket 且传入 `delimiter=/` 时，返回当前层直接对象和 `common_prefixes` 虚拟目录；目录模式按文件夹优先、Object Key 升序分页，Cursor 不得与普通列表混用。

### 10.5 删除

```text
DELETE /api/v1/media/{id}
```

删除接口只把对象变为 `delete_pending` 并返回 `202 Accepted`。Worker 负责：

```text
禁止新读取
-> 清理或移动原始文件
-> 使 Variant 失效
-> 请求 CDN Purge
-> 释放配额
-> 写入 media.deleted 事件
-> 标记 deleted
```

重复删除同一对象必须返回相同结果或成功状态，不能重复扣减配额。

首次进入 `delete_pending` 的事务同时写入 `media.delete_scheduled` OutboxEvent，并携带删除原因。Worker 完成物理删除后再写入 `media.deleted`。

物理删除完成时清除 Prompt 等用户 Metadata，只保留不含敏感信息的最小 Tombstone 供幂等判断和审计。Tombstone 到达配置的保留期后才从数据库清除。

### 10.6 内容读取

```text
GET  /{app_id}/{bucket}/{object_key}
HEAD /{app_id}/{bucket}/{object_key}
```

必须支持：

- `HEAD`
- 单段 `Range`；V1 对多段 Range 明确拒绝或按完整响应处理
- `ETag`、`If-None-Match`、`304 Not Modified`
- 正确的 `Content-Length`、`Content-Type` 和 `Accept-Ranges`
- 安全的 `Content-Disposition`

公开对象通过稳定的 Application、Bucket 和 Object Key 路径匿名读取。私有对象通过认证接口创建短时签名 URL，路径保持不变，仅增加 `token` 查询参数；Session 和 HMAC 凭证本身不会授权内容 URL。

Range 只适用于没有图片转换参数的原始对象。带 `w`、`h`、`format` 等转换参数的请求在 V1 始终返回完整派生内容和 `Accept-Ranges: none`；即使 Variant 已缓存，语义也保持一致。此类请求携带 Range Header 时忽略 Range 并返回 `200`。

### 10.7 批量操作

```text
POST /api/v1/media/batch
```

V1 支持对一组明确的 Media ID 批量执行：

```text
update_ttl_seconds
update_visibility
delete
```

- 所有 ID 必须属于当前认证的 Application。
- 单次同步操作限制对象数量，超过阈值时创建 AsyncJob 并返回 `202 Accepted`。
- 批量任务逐对象记录结果，部分失败不能伪装成全部成功。
- 按 Prefix 或 Bucket 筛选的大范围操作必须先生成预览数量，再显式确认异步执行。

## 11. 图片实时处理

示例：

```text
/{app_id}/{bucket}/{object_key}?w=600&h=600&fit=cover&quality=80&format=webp
```

V1 参数：

| 参数         | 示例     | 说明                         |
| ------------ | -------- | ---------------------------- |
| `w`          | `600`    | 目标宽度                     |
| `h`          | `600`    | 目标高度                     |
| `fit`        | `cover`  | `cover`、`contain`、`inside` |
| `quality`    | `80`     | 输出质量                     |
| `format`     | `webp`   | `jpeg`、`png`、`webp`        |
| `blur`       | `20`     | 模糊强度                     |
| `crop`       | `center` | 裁剪位置                     |
| `background` | `ffffff` | 填充背景色                   |

省略 `w` 和 `h` 时保持原图宽高，可仅通过 `format`、`quality`、`blur` 等参数转换编码或调整图像效果。只提供一个尺寸时按原图比例推导另一个尺寸。

安全限制：

- 限制输入文件大小、解码后总像素、输出宽高和总像素。
- 限制单请求处理时间、内存和并发数。
- 拒绝未知参数和超出范围的值。
- 私有原图生成的 Variant 必须继承私有权限。

### 11.1 缓存 Key

转换参数必须先规范化，例如排序参数、补充默认值并统一格式。缓存 Key：

```text
sha256(
  media.sha256
  + canonical_transform_parameters
  + processor_version
)
```

不能只使用 `sha256(query)`，否则不同原图会发生冲突，处理器升级后也可能继续返回旧结果。

并发生成相同 Variant 时需要使用数据库唯一约束或分布式锁避免缓存击穿。失败的生成不能留下可读取的半文件。

## 12. CDN 与缓存策略

文件内容不可变不代表所有对象都可以永久缓存。策略必须结合可见性和生命周期：

| 对象类型          | 建议 Cache-Control                                 |
| ----------------- | -------------------------------------------------- |
| 永久公开对象      | `public, max-age=31536000, immutable`              |
| 有 TTL 的公开对象 | `public, max-age=<不超过剩余 TTL>`                 |
| 私有对象          | `private, no-store`，或由受信 CDN 使用短时签名缓存 |
| Variant           | 继承原图权限和不超过原图的剩余生命周期             |

如果产品承诺立即撤销公开 URL，删除流程必须接入 CDN Purge。未配置 Purge 能力时，API 和 UI 应明确删除只保证源站不可访问，边缘缓存可能在 TTL 到期前继续存在。

## 13. 生命周期

生命周期规则属于 Bucket，也可由单个对象覆盖。

### 13.1 时间表示

所有 API 相对时长统一使用正整数秒，不接受 `30d`、`12h` 等字符串，也不提供固定枚举：

```text
default_ttl_seconds
ttl_seconds
duration_seconds
```

- 值必须是大于 0 的整数，并且不超过部署配置 `MAX_TTL_SECONDS`。
- JSON `null` 表示永不过期。
- 创建时省略 `ttl_seconds` 表示继承 Bucket 默认值；PATCH 时省略表示不修改。
- `expire_at` 表示 RFC 3339 绝对时间，不能与 `ttl_seconds` 同时提交。
- Bucket 在数据库保存 `default_ttl_seconds`；Media 保存计算后的 `expire_at`。
- Web UI 可以把秒数格式化成“30 天”等文本，但提交 API 时仍发送原始秒数。

### 13.2 规则类型

V1 支持两类独立规则：

```text
expire_after  按创建后的秒数触发
keep_latest  按数量保留
```

Bucket 策略示例：

```json
{
  \"rules\": [
    {
      \"id\": \"keep-recent-outputs\",
      \"enabled\": true,
      \"type\": \"keep_latest\",
      \"scope\": { \"prefix\": \"user-123/\" },
      \"count\": 10,
      \"action\": \"delete\"
    },
    {
      \"id\": \"expire-temp\",
      \"enabled\": true,
      \"type\": \"expire_after\",
      \"scope\": { \"prefix\": \"temp/\" },
      \"duration_seconds\": 604800,
      \"action\": \"delete\"
    }
  ]
}
```

`keep_latest` 的排序固定使用对象创建时间和 ID，范围固定为 Application、Bucket 和 Prefix。并发上传后由幂等 Worker 收敛，不能依赖单次非事务查询立即删除。`expire_after` 和 `keep_latest` 是不同规则类型，不能同时出现在同一条规则中。

### 13.3 后续规则

后续支持：

- `auto_archive`：先迁移到低频存储，经过一段时间再删除。
- `restore`：归档对象恢复后重新开放读取。
- `expire_after_download`：使用一次性下载令牌和完成确认实现。
- `expire_after_view`：依赖访问网关或 CDN 日志回传。

后两项不进入 V1。CDN 命中、Range、预加载、中断和重试都会让简单 HTTP 请求计数不可靠，不能直接把源站请求次数当成精确浏览或下载次数。

生命周期扫描只负责把对象推进到 `archive_pending` 或 `delete_pending`，不能直接执行 SQL DELETE。进入 `delete_pending` 时写入 `media.delete_scheduled`，TTL 触发时使用 `reason = ttl`；生命周期本身不增加 `expired` 状态。

## 14. 配额

配额以 Application 为主体：

```text
available = quota_bytes - used_bytes - reserved_bytes
```

要求：

- 上传开始前根据声明大小预占配额，实际流式大小超出限制时立即终止。
- 上传提交时将预占转为已用，失败或超时必须释放。
- 配额更新使用事务和条件更新，禁止并发超卖。
- V1 默认只统计原始对象；Variant 由系统缓存预算管理，不计入用户配额。
- `delete_pending` 在物理文件确认清理后释放配额。
- 归档对象仍计入配额，除非未来引入单独的存储等级计费模型。

## 15. Webhook

事件：

```text
media.uploaded
media.metadata_updated
media.archive_started
media.archived
media.restore_started
media.restored
media.delete_scheduled
media.deleted
```

事件流：

- 上传事务提交 `active` 时写入 `media.uploaded`。
- 任何原因首次进入 `delete_pending` 时写入 `media.delete_scheduled`，Payload 包含 `reason`。
- `reason` 支持 `manual`、`ttl`、`keep_latest`、`application_delete` 和 `admin`。
- TTL 不产生 `expired` 状态，也不单独发送过期事件。
- 物理文件、Variant、CDN 和配额清理完成后写入 `media.deleted`。
- 生命周期动作是归档时进入 `archive_pending` 并发送 `media.archive_started`，Payload 使用 `reason = ttl`。

请求头：

```text
X-MediaHub-Event-Id
X-MediaHub-Event-Type
X-MediaHub-Timestamp
X-MediaHub-Signature
```

要求：

- 使用每个 Endpoint 独立 Secret 对时间戳和原始 Body 签名。
- 至少一次投递，接收方使用 Event ID 幂等去重。
- 指数退避重试，并设置最大次数和死信状态。
- Web UI 可以查看投递历史、响应码并手动重放。
- Webhook URL 必须防御 SSRF，禁止访问本机、链路本地和内部保留网段，Self-hosted 可通过配置显式放行。

## 16. 一致性与后台任务

关系数据库和文件系统无法共享一个事务，因此必须明确最终一致性策略。

### 16.1 Reconciler

后台定期检查：

- 超时的 `uploading` 记录和临时文件。
- Storage 中存在但数据库无记录的孤儿文件。
- 数据库为 `active` 但 Storage 缺失的对象。
- 长时间停留在 `delete_pending`、`archive_pending` 的对象。
- 已删除 Media 遗留的 Variant。
- 超时未释放的配额预占。

发现数据缺失时不能静默修复并丢失证据，应记录审计事件和指标。

### 16.2 Worker 并发

- 任务领取必须有 Lease 和超时机制。
- 每种任务定义幂等键和状态转移前置条件。
- 多 Worker 同时处理同一任务时只有一个能提交状态变化。
- 外部调用失败保留错误摘要、重试次数和下次执行时间。

## 17. 文件与媒体安全

- 使用内容探测识别 MIME，不能信任扩展名和请求 `Content-Type`。
- Object Key 必须规范化并限制长度，拒绝 `..`、控制字符和非法编码。
- 原始文件物理路径不得由 Object Key 拼接产生。
- 限制压缩炸弹、超大像素图片和异常媒体头。
- 图片处理库和 FFmpeg 放入受限进程或容器，设置 CPU、内存和执行超时。
- 用户提供的文件名只用于展示或下载名，输出响应头前必须安全编码。
- 内容域统一返回 `X-Content-Type-Options: nosniff` 和 `Referrer-Policy: no-referrer`。
- HTML、SVG、XML、脚本及未知 MIME 默认使用 `Content-Disposition: attachment`，不能被控制台以内联方式打开。
- 只有明确允许的图片、音频、视频和 `application/pdf` 可以 `inline`。PDF 必须由隔离的内容域返回并设置严格的 `Content-Security-Policy: sandbox`；其他文档预览需要独立 Sandbox Origin。
- 控制台的文本、Markdown、DOCX、XLSX 和压缩包查看器必须在用户打开对应文件预览后按格式加载。压缩包只允许在独立 Worker 中列举目录，不得自动解压条目内容；压缩源文件、条目数、路径长度、单条及总声明大小、压缩比和执行时间都必须有硬上限，并在完成、失败、超时或关闭预览时终止 Worker。
- 对可能被浏览器执行的响应设置 `Content-Security-Policy: sandbox`，不得允许内容域读取控制台 Cookie。
- 私有对象的错误响应不能泄露对象是否存在，未授权时统一处理。
- Prompt、密钥、Session、签名 URL 和私有 Metadata 不进入普通访问日志。
- 所有管理操作记录 Audit Log，包括操作者、Application、动作、目标、时间和请求 ID。
- Cookie Session 的写操作必须防御 CSRF；跨域访问使用显式 CORS Allowlist，禁止默认反射任意 Origin。

## 18. API 约定

### 18.1 版本与格式

- 管理 API 使用 `/api/v1`。
- 成功和错误响应统一为 JSON，文件内容接口除外。
- 破坏性变化发布新的 API 主版本。
- 未知请求字段默认拒绝，避免拼写错误被静默忽略。

部署能力通过统一接口发现：

```text
GET /api/v1/capabilities
```

```json
{
  \"deployment_profile\": \"docker\",
  \"storage\": [\"local\", \"s3\"],
  \"s3_gateway\": true,
  \"image_processing\": true,
  \"video_processing\": false,
  \"resumable_upload\": false,
  \"archive_restore\": false
}
```

客户端只能根据能力字段决定 UI 和调用路径，不能通过版本号、域名或错误猜测当前运行平台。

错误示例：

```json
{
  \"error\": {
    \"code\": \"BUCKET_NOT_FOUND\",
    \"message\": \"Bucket does not exist\",
    \"request_id\": \"req_01J...\",
    \"details\": null
  }
}
```

### 18.2 常见状态码

| 状态码    | 含义                                   |
| --------- | -------------------------------------- |
| `200/201` | 查询、更新或创建成功                   |
| `202`     | 异步操作已接受                         |
| `204`     | 无响应体的成功操作                     |
| `400`     | 参数或签名格式错误                     |
| `401`     | 未认证或签名无效                       |
| `403`     | 已认证但没有权限                       |
| `404`     | 资源不存在，或私有资源对当前主体不可见 |
| `409`     | Object Key、Bucket 名称或幂等请求冲突  |
| `413`     | 文件或 Metadata 过大                   |
| `415`     | 不支持的媒体类型                       |
| `422`     | 媒体内容无效或策略校验失败             |
| `429`     | 超出速率限制或并发限制                 |

### 18.3 请求追踪

每个请求生成 `X-Request-Id`。调用方提供的 ID 只能在格式和长度通过校验后采用。请求 ID 必须贯穿日志、异步任务和 Webhook。

### 18.4 OpenAPI 与客户端生成

V1 以 Rust API DTO 和 `utoipa` 注解生成的 `openapi.json` 为规范来源：

```text
Rust DTO + utoipa
        -> openapi/openapi.json
        -> 生成 web/src/api/generated
        -> TanStack Query 封装
```

- 生成目录禁止手工修改。
- CI 重新生成 Spec 和 TypeScript Client，存在未提交差异时失败。
- Web UI、CLI、Docker 契约测试和未来 Cloudflare Runtime 使用同一份 Spec。
- Cloudflare Runtime 可以使用不同语言实现，但响应结构、错误码、权限和签名行为必须通过 OpenAPI 与行为契约测试。
- 破坏性 DTO 变化必须升级 API 主版本；兼容性检查在 CI 中执行。

## 19. Web UI

Web UI 位于仓库 `web/`，由 pnpm/Vite 在 Docker 构建阶段生成，并由 Axum 与 API 同源提供。前端只使用生成的 API Client，不直接拼接管理 API URL。

路由边界：

```text
/login
/register
/verify-email
/reset-password

/app/:appId/dashboard
/app/:appId/objects
/app/:appId/objects/:mediaId
/app/:appId/buckets
/app/:appId/access-keys
/app/:appId/webhooks
/app/:appId/settings

/admin/users
/admin/applications
/admin/storage
/admin/jobs
/admin/audit
```

- `/admin/*` 在路由加载和 API 两层检查 `system_role`。
- 应用启动先调用 `/auth/me` 和 `/capabilities`，再生成可用导航和操作。
- 生产构建默认调用浏览器当前 Origin；Vite 开发模式默认调用当前主机的 `3000` 端口。
- 控制台不包含 Mock API 或演示账号，前端构建环境不得包含 SecretAccessKey 或主密钥。
- React Router 只在明确的 Web 路径提供 SPA 回退，不代理或吞掉 API、WebDAV、S3 或原生媒体响应。
- TanStack Query 管理服务端状态；不使用 LocalStorage 保存 Session、Secret 或私有签名 URL。
- 上传管理器显示排队、进度、取消、重试和完成校验状态，刷新后可通过 UploadSession ID 恢复。
- Vitest、Testing Library 和 Playwright 覆盖认证、权限、密钥一次性展示、上传和批量删除流程。

V1 用户控制台包含：

- 邮箱注册、验证、登录、退出和密码重置
- Application 创建、切换、配额和用量
- AccessKey 创建、授权、轮换和撤销
- Bucket 创建、权限、大小限制和生命周期配置
- 对象列表、上传、预览、下载、Metadata 编辑和删除
- 图片 Variant 预览
- Webhook Endpoint、投递历史和重放

V1 系统后台包含：

- 用户和 Application 状态管理
- 删除队列、存储异常和后台任务状态
- 审计日志和 Webhook 失败诊断

Dashboard 指标：

- 对象总数、原始文件容量、今日上传和今日删除
- 各 Bucket 数量、容量和 MIME 分布
- 上传失败、图片处理失败、Webhook 积压和删除积压
- API 请求量、延迟、错误率和流量

“热门图片”只有在接入 CDN 日志或统一访问网关后才有准确意义。V1 如仅统计源站请求，UI 必须明确标注为源站访问量。

## 20. 可观测性

- 结构化日志：时间、级别、请求 ID、Application、接口、状态码、耗时。
- Metrics：请求延迟、吞吐、错误率、上传字节、磁盘容量、缓存命中率、任务积压。
- 健康检查：`/health/live` 和 `/health/ready`。
- Readiness 必须检查数据库和必要存储后端是否可用。
- 日志字段需要脱敏，禁止记录密码、Secret、Session、完整签名和 Prompt。
- 关键告警：存储空间不足、数据库不可写、任务持续积压、孤儿文件增长、Webhook 大量失败。

## 21. 备份与恢复

数据库和 Storage 必须作为同一个逻辑数据集备份。

PostgreSQL 与对象存储部署：

- 使用数据库时间点恢复能力和对象存储版本/快照策略。
- 记录备份批次 ID，恢复后运行 Reconciler。

密钥与凭证恢复：

- Keyring、数据库和 Storage 是同一个可恢复系统的三个组成部分，但必须分别加密和保管。
- 备份清单记录仍被密文引用的全部 Key Version，不能只备份 Active Key。
- 恢复演练必须验证 Session 签发、AccessKey HMAC、Webhook 签名和旧版本密文解密。
- 丢失所需 Master Key 时不得静默重置 Secret；服务进入 Fail Closed，并要求显式轮换受影响凭证。

必须定期演练恢复，而不是只确认备份文件存在。恢复流程需要验证对象数量、总字节、随机哈希抽检和关键权限。

## 22. Port 与 Adapter

Port 应围绕业务能力设计，不能建立一个包含所有云厂商行为的巨大 `CloudflareAdapter`。V1 定义接口并实现 Docker 所需 Adapter，未来 Cloudflare Profile 增加新的组合。

### 22.1 ObjectStore

接口至少包括：

```text
put
create_multipart
upload_part
complete_multipart
abort_multipart
open_range
head
delete
exists
list(prefix, cursor)
```

实现路线：

1. Local：V1 默认，适合单机 Docker。
2. S3 Compatible：MinIO、AWS S3、Cloudflare R2 等。
3. R2 Binding：未来 Cloudflare Worker 内部使用。
4. 云厂商适配：只有在通用 S3 接口不能满足时再增加 OSS、COS 专用实现。

数据库中保存 `storage_backend` 和不透明 `storage_key`，业务层不能依赖本地文件路径。

基础 Port 不提供 rename、move 或 trash 语义。对象从上传开始就使用最终 Storage Key，读取权限由 Media 状态控制。Local Adapter 可以内部使用临时文件、原子 rename 和 trash，但这些行为不能泄露给 Core。

`head` 返回大小、内容类型、ETag、可用 Checksum 和 Provider Metadata；调用方必须区分 ETag 与内容 Checksum。`list` 必须使用 Prefix 和 Cursor 分页，Reconciler 不得假设可以一次枚举整个 Bucket。

### 22.2 Repository

Repository 按聚合和 Use Case 定义，例如 Media、Bucket、Quota、AsyncJob 和 Outbox，不暴露 SQL、SQLx 类型或数据库连接。所有运行时统一使用 PostgreSQL SQLx Adapter。

Repository 用于隔离应用服务与 SQLx 查询实现，不承担数据库切换。事务边界和并发语义由 Use Case 明确声明，并由 PostgreSQL 实现和集成测试验证。

### 22.3 ImageProcessor

```text
Docker Profile:      libvips
Cloudflare Profile:  Cloudflare Images/Transformations
```

Core 只定义规范化转换参数和结果，不依赖具体处理器。`processor_version` 必须包含实现与版本，防止不同处理器复用不兼容的 Variant。

FFmpeg 不适合 Cloudflare Workers。视频处理在 Cloudflare Profile 中可以关闭，或未来接入独立媒体处理服务。

### 22.4 JobDispatcher

```text
Docker Profile:      Database Queue + Tokio Worker
Cloudflare Profile:  Queues + Cron Triggers + Workflows
```

任务消息只包含稳定 ID、任务类型和幂等键，不序列化运行时对象。任务状态仍以 Repository 中的业务记录为准，消息队列只负责触发处理。

### 22.5 EventPublisher

Docker Profile 使用 PostgreSQL Outbox Worker；Cloudflare Profile 从同一个 PostgreSQL Outbox 投递到 Queues。两者都必须提供至少一次投递、稳定 Event ID 和相同 Webhook 签名格式。

### 22.6 Adapter 契约

每个 Port 都必须有与运行时无关的契约测试，至少覆盖：

- 幂等写入和重复删除。
- 条件更新与并发冲突。
- Range 和对象不存在语义。
- 上传提交前后的可见性。
- 任务重试和重复消息。
- 错误码到领域错误的稳定映射。

## 23. 数据库支持

- PostgreSQL 17：唯一支持的数据库，适用于单机、多实例和未来的边缘运行时部署。

为保持可移植性：

- 核心查询避免依赖数据库特有 JSON 查询。
- 时间、布尔值、枚举和 UUID 的编码规则统一封装。
- 多实例 Worker 只在数据库能够提供可靠任务领取语义时启用。
- 每次数据库迁移必须提供升级测试；破坏性迁移前必须备份。

## 24. 部署

MediaHub 定义两个 Deployment Profile。V1 只交付 Docker Profile，但代码边界必须持续满足 Cloudflare Profile 的可实现性。

### 24.1 Docker Profile（V1）

V1 提供 Docker Compose：

```bash
docker compose up -d
```

推荐服务：

```text
mediahub       Web、API 与 Worker
postgres       必需，PostgreSQL 17 Metadata 数据库
reverse-proxy  可选，TLS、限流和 CDN 回源
```

关键配置类别：

```text
MEDIAHUB_DATABASE_URL
MEDIAHUB_STORAGE_BACKEND
MEDIAHUB_STORAGE_ROOT
MEDIAHUB_ACCESS_KEY_MASTER_KEY
MEDIAHUB_ACCESS_KEY_MASTER_KEY_VERSION
MEDIAHUB_MEDIA_SIGNING_KEY
MEDIAHUB_RESEND_API_KEY
MEDIAHUB_EMAIL_FROM
MEDIAHUB_WEB_URL
MEDIAHUB_CORS_ALLOWED_ORIGINS
MEDIAHUB_REGISTRATION_ENABLED
```

生产环境要求：

- TLS 终止和可信代理配置正确。
- PostgreSQL、Storage 和主密钥使用持久化安全挂载。
- 容器以非 root 用户运行。
- `/storage` 不由普通静态文件服务器绕过 MediaHub 直接暴露。
- 配置文件和日志不包含 SecretAccessKey 或主密钥。

Docker Profile 可以使用 R2 的 S3 Compatible API，因此推荐的早期生产组合是：

```text
Docker              Axum Web + API + Worker
PostgreSQL          Metadata
Cloudflare R2       ObjectStore
Cloudflare CDN      Content Delivery
```

### 24.2 Cloudflare Native Profile（未来）

```text
Static Web hosting（future）     Web UI
Cloudflare Workers               API Runtime
PostgreSQL（通过 Hyperdrive）    Repository
Cloudflare R2                    ObjectStore
Queues + Cron + Workflows        JobDispatcher
Cloudflare Images                ImageProcessor
```

该 Profile 使用独立 Runtime Entry 和 Adapter，不尝试把 Axum Docker 二进制直接部署到 Workers。Rust Worker 是否用于生产，应在实施阶段根据 Cloudflare 当时的 Rust/WASM 支持重新评估；必要时 Worker Runtime 可以使用 TypeScript，但必须遵守同一 OpenAPI、签名规范和契约测试。

Cloudflare Profile 可根据平台限制关闭 FFmpeg 视频处理等能力。API 必须通过 Capability 响应声明当前部署支持的功能，Web UI 据此隐藏或禁用不可用操作。

Cloudflare V2 数据与任务策略：

- 初始实现使用单个 PostgreSQL Database 作为一个 MediaHub 部署的 Metadata Store，所有主键必须全局唯一且不能依赖数据库自增序列。
- Application ID 是未来数据库路由键。达到容量或吞吐阈值后才能引入按 Application 分片；分片前必须单独设计 Control Plane 映射、迁移和跨分片管理查询。
- PostgreSQL 的状态变化和 Outbox 行在同一个事务中写入。Cron/Worker 使用 Lease 分批读取 Outbox、发送 Queue 后标记结果；崩溃造成的重复消息由幂等 Consumer 处理。
- Queue 消息不是业务事实来源，PostgreSQL 中的 Media、UploadSession、AsyncJob 和 Outbox 状态才是事实来源。
- R2 对账优先依据 UploadSession、稳定 Storage Prefix、R2 Event Notification 和分页清单；事件通知允许重复或延迟，不能作为唯一事实来源。
- Reconciler 禁止在普通周期任务中全量扫描 R2 Bucket。全量 Inventory 属于低频运维任务，必须分页、限速并记录扫描游标。
- PostgreSQL 连接、查询、事务以及 Worker CPU/内存和 Queue 限制不得硬编码为永久常量；部署时读取配置并在 Metrics、Readiness 和 Capability 中暴露接近限制的状态。
- PostgreSQL WAL/PITR 与 R2 版本、快照和灾难恢复必须在启用 Cloudflare Profile 前形成独立操作手册。

### 24.3 域名与前端部署

推荐域名：

```text
mediahub.example.com    内置 Web 与管理 API
cdn.example.com         可选的用户媒体内容域，不携带控制台 Cookie
```

默认内置 Web 与 API 同源，不需要 CORS。原生对象响应会对主动内容启用 CSP sandbox、`nosniff` 和下载策略；对隔离要求更高的部署仍应使用独立内容/CDN 域，私有媒体预览使用短时签名 URL。

## 25. 技术栈

Backend：

- Rust
- axum
- tokio
- serde
- sqlx
- utoipa

Database：

- PostgreSQL 17

Image：

- libvips-rs 或成熟的 libvips Rust Binding

Video：

- FFmpeg，仅用于后续封面、截图和转码

Cache：

- moka：进程内 Metadata 缓存
- Local/S3：Variant 二进制缓存

Frontend：

- Vite
- React
- TypeScript
- HeroUI
- Tailwind CSS
- React Router
- TanStack Query
- React Hook Form + Zod
- OpenAPI Generated Client
- Vitest + Testing Library

Deploy：

- Docker
- Docker Compose
- Cloudflare Native Profile（后续）

## 26. 分阶段路线图

### V1：可靠的媒体对象存储

- 邮箱注册、验证、登录、密码重置
- Application、AppId、AccessKey 和 HMAC
- Bucket 和权限策略
- 原子上传、对象列表、Metadata、下载和 Range
- 公开、私有和预签名 URL
- 图片 Resize、格式转换和缓存
- TTL、`keep_latest`、异步删除
- 配额、Webhook Outbox、Audit Log
- Local Storage、PostgreSQL
- S3 Compatible Port 预留 R2，并保持 Core 不依赖 Docker Runtime
- 基础 Web UI、指标、备份和 Reconciler

### V1.1：生产能力

- S3 Compatible Storage，包括 Docker 后端连接 R2
- CDN Purge Adapter
- 多实例 Worker
- 大文件分片和断点续传
- Webhook 投递管理
- 更完善的速率限制和存储完整性扫描

### V2：Cloudflare Native Profile

- Cloudflare Workers Runtime Entry
- 通过 Hyperdrive 访问 PostgreSQL Repository
- R2 Binding ObjectStore Adapter
- Queues、Cron 和 Workflows JobDispatcher
- Cloudflare Images Adapter
- Adapter 契约测试和 Docker/Cloudflare API 一致性测试
- Capability API 和功能降级 UI

### V2.1：媒体处理与归档

- 视频封面和指定时间截图
- GIF 片段和视频转码
- `auto_archive` 和恢复流程
- CDN 日志导入和访问分析
- 一次性下载令牌
- Application 内可选的同内容去重；不得跨 Application 泄露哈希命中信息

### V3：可选 AI 媒体能力

- NSFW 检测
- OCR、Caption、Tag
- Embedding 和相似图片搜索
- Watermark
- Metadata 索引

这些能力只产生或索引对象 Metadata，不把 MediaHub 变成 AI 生成任务平台。

## 27. V1 验收标准

V1 至少满足：

- 任意上传失败或进程中断不会永久占用配额，临时与孤儿文件可被回收。
- 不同 Application 之间无法通过对象 ID、Bucket 名称或签名越权访问。
- 文件内容不可覆盖，重复 Object Key 返回冲突。
- 私有对象和私有 Variant 不会被匿名访问或公开缓存。
- TTL 删除会清理原文件、Variant、配额和事件，并能从中途失败恢复。
- 图片缓存不会在不同原图或处理器版本之间冲突。
- AccessKey 可独立授权、过期、撤销和轮换，Secret 只展示一次。
- Metadata 的系统字段不可伪造，Prompt 不通过公开内容接口泄露。
- HMAC Mutation 请求不能在签名时间窗口内被重复执行，合法重试使用新 Nonce 和原 Idempotency-Key 恢复结果。
- UploadSession 能在完成、取消、过期和校验失败后正确提交或释放配额，Multipart ETag 不被误认为内容哈希。
- Webhook 不会因服务崩溃永久丢失，重复投递有稳定 Event ID。
- 批量操作可追踪每个对象的结果，失败后能够安全重试。
- 数据库与 Storage 可以备份、恢复，并通过 Reconciler 完成一致性检查。
- Core 和 Application Crate 不依赖 Axum、SQLx、本地文件系统、libvips 或 Cloudflare Binding。
- Local 与 S3 ObjectStore 通过同一套契约测试，为未来 R2 Binding 保留稳定语义。
- OpenAPI、HMAC 签名和领域错误不包含部署平台特有字段。
- OpenAPI 和生成的 TypeScript Client 由 CI 验证无漂移。
- Master Keyring 缺失时服务 Fail Closed，备份恢复可以验证旧版本密文和签名。

## 28. 后续设计文档

实现前还应分别补充：

- API OpenAPI 规范
- HMAC Canonical Request 与签名测试向量
- 数据库迁移和索引设计
- Storage Backend Trait 与错误语义
- Repository、ImageProcessor、JobDispatcher 和 EventPublisher Port 契约
- Docker 与 Cloudflare Deployment Profile 能力矩阵
- Cloudflare Native 的 PostgreSQL/Hyperdrive、R2、Queues、Images 映射与限制
- 生命周期规则 JSON Schema
- 权限矩阵
- 威胁模型与安全测试清单
- Docker Compose、配置参考和备份恢复手册
