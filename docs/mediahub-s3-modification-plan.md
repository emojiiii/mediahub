# PrismArk S3 对齐源码修改方案

> 文档性质：代码修改文档，不是产品规划或架构白皮书
> PrismArk 工作区：`E:\emojiiii\mediaHub`
> Silo 参考源码：`E:\emojiiii\mediaHub\.research\silo`（已由根 `.gitignore` 排除）
> Silo 版本：`100e2e57a799781d23f134acaf47ce454a468be6`
> 前提：目前没有线上用户和需要保留的数据，最终版本不保留旧 schema、旧 API、旧权限或兼容层
> 实施原则：开发期间按可验证的纵向切片迁移；所有消费者切换完成后，再删除旧模型并压平为新的 `0001_initial.sql`

## 0. 2026-08-08 实施快照

本文同时记录最终目标和具体修改路径。当前仓库已经完成 S3 核心纵向闭环，但尚未达到“完整 S3 兼容”或本文第 22 章的最终清理标准。

| 切片 | 当前状态 | 已落地代码与行为 | 下一步 |
| --- | --- | --- | --- |
| S3 入口 | 已完成 | 独立 `MEDIAHUB_S3_BIND_ADDR` listener，默认 `0.0.0.0:9000`；Bucket/Object 使用根路径；生产 Router 已删除 `/s3` 路由 | 增加反向代理、virtual-host style 与更多客户端矩阵 |
| 对象模型 | 核心完成 | `objects`、不可变 `object_versions`、`upload_intents`、`storage_gc_tasks`；S3、WebDAV 普通文件路径和版本预览后端不读写 `Media` | 切换 JSON 写路径、Variant 等剩余消费者并删除旧模型 |
| Versioning | 核心完成 | Unversioned/Enabled/Suspended、opaque version ID、null version、delete marker、精确版本读删、ListObjectVersions、当前指针重算 | 控制台版本历史与更多客户端矩阵 |
| Put/Get/Head/Copy | 核心完成 | SigV4 顺序、流式写入、SHA-256 + MD5、Content-MD5、单 Range、条件请求、metadata、版本响应头、CopyObject | 更多 checksum；外部 S3 Full GET/Copy 改成真正端到端流式读取 |
| List/Delete | 核心完成 | ListObjectsV2 与 ListObjectVersions 的分页/marker/prefix/delimiter；DeleteObject/DeleteObjects 的 marker/null/lock/GC 语义 | ListObjectsV1 与更多 golden/客户端测试 |
| Multipart | 核心完成 | Create/UploadPart/ListParts/Complete/Abort/ListMultipartUploads/UploadPartCopy；Part MD5、标准 multipart ETag、恢复与重放、ObjectVersion 原子提交、持久 GC | 真实客户端并发与故障注入矩阵 |
| Application Quota | 部分完成 | JSON/原生上传链路已有 `used_bytes/reserved_bytes` 账本 | S3 Put/Multipart/Copy/覆盖/null replacement/永久删除必须作为独立纵向切片一次性接入，禁止只在 Lifecycle 删除侧扣减 |
| Bucket 配置 | 部分完成 | List/Create/Head/Delete/GetLocation；Versioning GET/PUT；Lifecycle GET/PUT/DELETE；Object Lock GET/PUT/CreateBucket header 与不可逆约束 | Policy、CORS、Notification、Bucket Tagging |
| Lifecycle | 核心完成 | 配置 schema/parser/validator；ObjectVersion Expiration、Noncurrent Expiration、Expired Marker、Multipart Abort；事务复检、Object Lock 与持久 GC | 标签/大小过滤、Transition、性能与多实例 soak 矩阵 |
| Object Lock | 核心完成 | Bucket 配置、默认 Retention、PutObject 锁头、对象 Retention/Legal Hold GET/PUT、签名 Governance bypass、不可逆 Versioning 约束与删除事务保护 | CopyObject/Multipart 显式锁头支持与 native-client 矩阵 |
| Object Tagging | 核心完成 | 当前/精确版本 GET/PUT/DELETE；独立版本标签表；PutObject、Copy COPY/REPLACE、Multipart 冻结与 TagCount | Bucket Tagging、标签条件 Policy/Lifecycle、真实 AWS CLI endpoint 回归 |
| Policy/Auth | 未完成 | 现有 SigV4 和 Application 授权边界继续工作 | 替换固定权限为标准 S3 Policy evaluator |

本轮没有加入旧 `/s3`、旧 schema 或旧品牌兼容代码。历史 `Media` 路径仍存在只是因为非 S3 消费者尚未全部迁移，不是新旧双写；S3 普通对象与 Multipart 的新写入只提交到 Object/ObjectVersion。

### 0.1 真实后端验证记录

- PostgreSQL 17 fresh migration：通过。
- PostgreSQL Repository Contract：1/1 通过。
- Server 全量测试：lib 8/8、server 165/165 通过，包含真实 PostgreSQL 的 SigV4 S3、WebDAV、Object Tagging 和不可变版本预览用例。
- PostgreSQL Repository、S3 列表、Bucket Object Lock、ObjectVersion Lock 与 Object Tagging 合同：各 1/1 通过。
- PostgreSQL Lifecycle 合同 1/1、App 52/52 通过；覆盖 Enabled/Suspended/Unversioned、配置与 head 竞争、Retention、GC、Multipart 锁序、公平预算和普通 Expiration marker。
- Silo `docker.io/pgsty/silo:latest`：真实 ObjectStore 与 Presigned PUT 合同 1/1 通过。
- 真实测试修复了两个仅靠静态检查无法发现的问题：PostgreSQL constraint 自动命名冲突/NUL 检查，以及 generic S3 create-only copy 的能力差异。

## 1. 本次修改的最终形态

本次修改完成后：

- S3 使用独立 endpoint，根路径就是标准 S3 API，不再使用 `/s3` 前缀。
- S3、WebDAV、JSON API、Web UI 都调用同一个 `ObjectService`。
- Bucket + Key 与对象内容版本彻底分离。
- `objects` 保存逻辑 Key 和当前版本指针。
- `object_versions` 保存不可变内容、ETag、checksum、metadata、delete marker 和 retention。
- 同 Key PUT 生成新版本并原子切换当前指针。
- S3 Versioning、Delete Marker、CopyObject、条件请求和 Multipart 都在 `ObjectService` 中实现。
- WebDAV 只是 S3 对象语义的路径适配器，不再直接操作 `MediaRepository`。
- 预览和图片 Variant 都绑定 `object_version_id`，不会因 Key 被覆盖而内容漂移。
- 当前固定字符串权限全部删除，Access Key 直接使用 S3 Policy。
- 当前自定义 `ExpireAfter` / `KeepLatest` 生命周期全部删除，改用 S3 LifecycleConfiguration。

本轮不做：

- Silo 的纠删码、Healing、Rebalance、Decommission；
- Bucket/Site Replication；
- STS、OIDC、LDAP；
- SSE-KMS/SSE-C；
- SelectObjectContent；
- Website、Accelerate、RequestPayment；
- 视频 Variant 和外部视频队列。

## 2. 从 Silo 源码参考什么

Silo 是 AGPL-3.0-or-later。PrismArk 是 MIT 项目，因此：

- 可以参考 S3 行为、模块边界、handler 顺序、测试案例和公开协议；
- 可以对照 Silo 做黑盒兼容测试；
- 不应直接复制 Silo/MinIO 的非平凡 Go 实现到 PrismArk；
- 本文中的“参考”均指依据公开协议重新用 Rust 实现相同行为，不复制 Silo/MinIO 代码。

| Silo 文件 | 参考内容 | PrismArk 目标 |
|---|---|---|
| `cmd/api-router.go` | 按 method/path/query/header 区分 S3 operation | `mediahub-server/src/s3/operation.rs` |
| `cmd/object-api-interface.go` | Handler 与 ObjectLayer 分离 | `mediahub-app/src/object_service.rs` |
| `cmd/object-api-options.go` | 集中解析 version/checksum/conditions | `mediahub-server/src/s3/operation_context.rs` |
| `cmd/object-handlers.go` | Get/Head/Put/Copy/Delete 校验顺序 | `s3/handlers/object.rs` |
| `cmd/object-multipart-handlers.go` | Multipart 初始化时冻结 metadata/lock/checksum | `s3/handlers/multipart.rs` |
| `cmd/bucket-versioning-handler.go` | Versioning XML、权限、状态约束 | `s3/handlers/versioning.rs` |
| `internal/bucket/versioning` | Enabled/Suspended/null version 判断 | `mediahub-core/src/versioning.rs` |
| `cmd/bucket-object-lock.go` | Legal Hold、Governance、Compliance、bypass | `mediahub-core/src/object_lock.rs` |
| `internal/bucket/lifecycle` | Rule parser、filter、evaluator 分离 | `mediahub-core/src/lifecycle/` |
| `cmd/auth-handler.go`、`cmd/iam.go` | 签名认证与 Policy 授权分离 | `s3/auth.rs` + `mediahub-core/src/policy/` |
| `cmd/api-errors.go`、`cmd/api-response.go` | 集中 S3 error code 和 XML response | `s3/error.rs`、`s3/response.rs` |
| `cmd/event-notification.go`、`internal/event` | 事件名、filter、target 分离 | 现有 Outbox/Webhook 扩展 |
| `cmd/signature-v4*_test.go` | SigV4/Presigned/Streaming 测试分类 | `tests/s3/sigv4.rs` |
| `cmd/object-api-listobjects_test.go` | List/Versions/Delete Marker/分页 | `tests/s3/listing.rs` |
| `cmd/object-api-multipart_test.go` | Multipart 生命周期和边界 | `tests/s3/multipart.rs` |

不要参考或移植：

- `cmd/erasure*.go`
- `cmd/format-erasure*.go`
- Healing/Scanner/Rebalance/Decommission
- Site Replication 和 peer internode API
- `.minio.sys` 磁盘格式

这些属于 Silo 的存储内核，不属于 PrismArk。

## 3. 最终切换后删除的代码

本章列的是**最终仓库形态**，不是开发第一步。任何旧文件或旧类型只有在以下条件全部满足后才能删除：

1. 对应的新领域模型、Repository Contract 和协议测试已经通过；
2. S3、JSON API、WebDAV、Preview、Variant、Short Link、Batch Job 与 Worker 等消费者均已切换；
3. 不再存在旧 `Media` 真相源，也不存在新旧模型 dual-write；
4. fresh install 和完整回归测试通过。

开发阶段允许新旧代码短暂并存以建立纵向闭环，但每个写入口只能有一个真相源；最终发布产物不保留兼容路由、兼容列、旧 ID 映射或双写逻辑。

### 3.1 删除旧 S3 HTTP 实现

完成新 `s3/` 模块后删除：

- `crates/mediahub-server/src/s3_http.rs`
- `crates/mediahub-server/src/s3_http_core.rs`
- `crates/mediahub-server/src/s3_http_support.rs`
- `crates/mediahub-server/src/s3_http_multipart.rs`

原因：

- 当前 PUT/GET/POST/DELETE handler 内部手工判断 query；
- Bucket、Object、ACL、Multipart 混在同一入口；
- 每个 handler 直接调用旧权限字符串和 `MediaRepository`；
- `versionId` 被统一拒绝；
- 继续补丁式扩展会让每个新 subresource 都增加分支。

保留并搬迁：

- `s3_gateway.rs` 中 SigV4 parser/verifier，搬到 `s3/auth/sigv4.rs`；
- `s3_list.rs` 中 V2 query/token/XML，搬到 `s3/xml/list.rs`；
- `s3_xml.rs` 中 DeleteObjects 和 Multipart XML，拆到 `s3/xml/`；
- `s3_multipart_storage.rs` 中临时 storage key 生成逻辑，搬到 app/storage 层。

### 3.2 删除 `versionId` 拒绝逻辑

删除：

- `s3_http_support.rs::reject_s3_versioning`
- 所有调用点；
- `tests.rs` 中断言 `versionId` 返回 `NotImplemented` 的测试。

替换为：

- `S3OperationContext.version_id: Option<S3VersionId>`
- `ObjectService::resolve_version`
- 正常版本读取、删除和 tagging 流程。

### 3.3 删除固定权限模型

删除：

- `crates/mediahub-server/src/main.rs::ACCESS_KEY_PERMISSIONS`
- `access_keys.rs` 中对九个固定权限字符串的校验；
- OpenAPI 的固定 `Permission` enum；
- `ApplicationAuth::authorize("media:read")` 等调用；
- WebDAV 的 `require("media:*")` / `require_any`；
- Access Key 创建页面的固定权限复选框。

Access Key 表删除 `permissions JSONB`，改为：

- `policy_document JSONB NOT NULL`
- `policy_revision BIGINT NOT NULL`

所有授权只调用：

```text
PolicyEvaluator::authorize(principal, action, resource, conditions)
```

Application ID 是硬租户边界。任何 policy 都不能授权访问其他 Application。

### 3.4 删除旧 Media 模型

删除或重写：

- `crates/mediahub-core/src/media.rs`
- `Media`
- `NewMedia`
- `PersistedMedia`
- `MediaState` 中与对象版本无关的 mutable content 状态；
- `metadata_version`
- `revision`
- `Media::update_*`
- `MediaRepository`
- `memory_media.rs`
- PostgreSQL `media.rs`、`media_queries.rs`、`media_mutations.rs` 中以 media 为逻辑 Key 的查询。

尤其删除：

- `UNIQUE(application_id, bucket_id, object_key)`
- `Media::new` 中 `etag = sha256`
- `find_by_object_key -> Media` 的单行模型。

### 3.5 删除旧上传应用服务

删除：

- `UploadMediaService`
- `UploadMediaRequest`
- `StagedUploadMediaRequest`
- `upload_staged`
- `upload_multipart_staged`
- 以 `MediaRepository::commit_upload` 为中心的提交流程。

`UploadSessionService` 也不再创建 Media。它只负责：

1. 创建 staged upload；
2. 返回上传目标；
3. 完成时调用 `ObjectService::commit_staged_put`。

### 3.6 删除旧 Bucket 生命周期

删除：

- `LifecycleRule::ExpireAfter`
- `LifecycleRule::KeepLatest`
- 相关 JSON DTO、UI 表单、worker 判断和测试。

只保留 S3 Lifecycle：

- Expiration；
- NoncurrentVersionExpiration；
- ExpiredObjectDeleteMarker；
- AbortIncompleteMultipartUpload；
- Prefix/Tag filter。

### 3.7 删除 `/s3` 路由

从 `crates/mediahub-server/src/http.rs` 删除：

- `/s3/{bucket}`
- `/s3/{bucket}/{*object_key}`

新增独立 S3 Router：

- `GET /`：ListBuckets
- `PUT /{bucket}`：CreateBucket
- `HEAD /{bucket}`：HeadBucket
- `DELETE /{bucket}`：DeleteBucket
- `GET /{bucket}`：Bucket operations
- `PUT|GET|HEAD|POST|DELETE /{bucket}/{*key}`：Object operations

Web UI/API 继续走原主站 listener。S3 使用：

- 独立端口；或
- 独立 `s3.example.com` hostname。

推荐独立 listener，避免 Axum UI fallback、CORS 和 body limit 干扰 S3。

### 3.8 删除 WebDAV FakeLs

删除：

```rust
.locksystem(FakeLs::new())
```

删除当前 WebDAV 直接依赖：

- `MediaRepository`
- `UploadMediaService`
- 旧固定 permission strings。

重写 WebDAV adapter 后再删除旧 `webdav_fs.rs` 和 `webdav_file.rs` 的 media-specific 实现。

## 4. 数据库模型与最终基线

最终发布仍采用“不兼容旧版本”的干净基线：不做线上 backfill、dual write、shadow read、compatibility view、legacy column 或旧 Media ID 映射。但**不能在开发第一步删除现有 `0001-0011` 和 `Media`**。

开发阶段采用以下迁移纪律：

1. 新增临时开发迁移，建立 `objects`、`object_versions`、Bucket 配置、Upload Intent 和 GC Task；
2. 用 Repository Contract、并发测试和 HTTP 纵向切片验证模型；
3. 逐个切换 S3、Multipart、JSON API、WebDAV、Preview、Variant、Short Link、Batch Job 与 Worker；
4. 每个写入口切换后立即停止旧模型写入，禁止长期 dual-write；
5. 所有消费者切换且全量测试通过后，删除 `Media` 及旧迁移；
6. 最后压平为新的 `0001_initial.sql`，通过 fresh install 验证后发布。

临时开发迁移只服务于实现和验证，不进入最终发布基线。由于没有线上用户和保留数据，最终切换无需兼容迁移，但开发过程必须保留可定位、可回归的阶段边界。

### 4.1 `applications`

保留现有 Application、quota 和管理字段。Quota 建议拆为：

- `current_object_bytes`
- `noncurrent_version_bytes`
- `variant_cache_bytes`
- `reserved_upload_bytes`

如果暂时不分开计费，至少保留：

- `used_bytes`
- `reserved_bytes`
- `quota_bytes`

`used_bytes` 必须明确包含 current 与 noncurrent data version，但不包含待 GC 且已完成逻辑删除的 blob；预留额度只记在 `reserved_bytes`。

### 4.2 `access_keys`

```sql
CREATE TABLE access_keys (
    id UUID PRIMARY KEY,
    application_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    access_key_id TEXT NOT NULL UNIQUE,
    secret_ciphertext TEXT NOT NULL,
    secret_key_version INTEGER NOT NULL,
    secret_last_four TEXT NOT NULL,
    name TEXT NOT NULL,
    policy_document JSONB NOT NULL,
    policy_revision BIGINT NOT NULL DEFAULT 1,
    expires_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);
```

### 4.3 `buckets` 与 Versioning 状态机

```sql
CREATE TABLE buckets (
    id UUID PRIMARY KEY,
    application_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    region TEXT NOT NULL,
    versioning_status TEXT NOT NULL DEFAULT 'unversioned'
        CHECK (versioning_status IN ('unversioned', 'enabled', 'suspended')),
    object_lock_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    object_ownership TEXT NOT NULL DEFAULT 'bucket-owner-enforced',
    max_object_bytes BIGINT,
    allowed_mime_types JSONB NOT NULL DEFAULT '[]',
    configuration_revision BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE(application_id, name),
    UNIQUE(id, application_id),
    CHECK (object_lock_enabled = FALSE OR versioning_status = 'enabled')
);
```

状态必须使用 `Unversioned / Enabled / Suspended`，不能使用 `Disabled` 混淆“从未启用”和“已启用后暂停”：

- 新 Bucket 默认 `Unversioned`；
- `Unversioned -> Enabled` 合法；
- `Enabled <-> Suspended` 合法；
- 一旦进入 `Enabled`，永远不能回到 `Unversioned`；
- 启用 Object Lock 时，在同一事务内把 Versioning 设置为 `Enabled`；
- Object Lock 启用后不可关闭，且该 Bucket 禁止切换为 `Suspended`；
- Object Lock 可以在 Bucket 创建后启用，不要求只能创建 Bucket 时决定。

上述状态转换由 `BucketConfigurationRepository` 在事务中校验；不能只依赖 HTTP handler，也不能靠数据库 CHECK 表达全部历史状态。

### 4.4 Bucket 配置表

第一阶段只建立当前闭环需要的配置表，避免先引入一个混合 Policy/CORS/Notification/Lock 的大 JSON：

```sql
CREATE TABLE bucket_lifecycle_configurations (
    bucket_id UUID PRIMARY KEY REFERENCES buckets(id) ON DELETE CASCADE,
    configuration JSONB NOT NULL,
    revision BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE bucket_object_lock_configurations (
    bucket_id UUID PRIMARY KEY REFERENCES buckets(id) ON DELETE CASCADE,
    default_retention_mode TEXT
        CHECK (default_retention_mode IN ('governance', 'compliance')),
    default_retention_days INTEGER,
    default_retention_years INTEGER,
    revision BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    CHECK (default_retention_days IS NULL OR default_retention_years IS NULL)
);
```

Object Lock 可以启用但不设置默认 Retention。Bucket Policy、CORS、Notification 与 Tagging 在各自纵向切片中独立建表或建明确类型，不阻塞 Versioning/Object Lock/Lifecycle 的第一阶段闭环。

### 4.5 `objects`

`objects` 一行只表示一个逻辑 Bucket + Key，不保存 ETag、size、storage key 或删除状态：

```sql
CREATE TABLE objects (
    id UUID PRIMARY KEY,
    application_id UUID NOT NULL,
    bucket_id UUID NOT NULL,
    object_key TEXT NOT NULL,
    current_version_id UUID,
    generation BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE(application_id, bucket_id, object_key),
    UNIQUE(id, application_id, bucket_id),
    FOREIGN KEY(bucket_id, application_id)
        REFERENCES buckets(id, application_id)
);
```

规则：

- `object_key` 大小写敏感，不折叠重复斜杠；
- 目录是 Key prefix 投影，不是隐式数据库对象；显式零字节目录对象仍按普通 Key 处理；
- `current_version_id` 可以指向 data version 或 delete marker；
- `generation` 每次 current head 变化递增；
- head 切换必须锁定 Object 行或使用 generation CAS；
- 不存 `is_latest`，由 `objects.current_version_id` 推导；
- 历史清空后的空 `objects` 行可以暂留，再由轻量清理任务回收。

```sql
CREATE INDEX objects_list_idx
ON objects(application_id, bucket_id, object_key);
```

### 4.6 不可变 `object_versions`

只有已经提交、可参与对象历史的 data version 或 delete marker 才能进入 `object_versions`。Upload Intent、普通 staged upload、Multipart Upload 和 Part 均不进入版本历史。

```sql
CREATE TABLE object_versions (
    id UUID PRIMARY KEY,
    object_id UUID NOT NULL,
    application_id UUID NOT NULL,
    bucket_id UUID NOT NULL,

    external_version_id TEXT NOT NULL,
    generation BIGINT NOT NULL,
    version_kind TEXT NOT NULL
        CHECK (version_kind IN ('data', 'delete_marker')),

    storage_backend TEXT,
    storage_key TEXT,
    provider_etag TEXT,
    provider_version TEXT,

    etag TEXT,
    size_bytes BIGINT,
    content_type TEXT,
    checksum_md5 TEXT,
    checksum_sha256 TEXT,
    checksum_crc32 TEXT,
    checksum_crc32c TEXT,

    cache_control TEXT,
    content_disposition TEXT,
    content_encoding TEXT,
    content_language TEXT,
    http_expires_at TIMESTAMPTZ,
    user_metadata JSONB NOT NULL DEFAULT '{}',
    ai_metadata JSONB NOT NULL DEFAULT '{}',

    became_noncurrent_at TIMESTAMPTZ,
    retention_mode TEXT
        CHECK (retention_mode IN ('governance', 'compliance')),
    retain_until_at TIMESTAMPTZ,
    legal_hold BOOLEAN NOT NULL DEFAULT FALSE,
    lock_revision BIGINT NOT NULL DEFAULT 0,

    created_by TEXT NOT NULL,
    source_protocol TEXT NOT NULL
        CHECK (source_protocol IN ('s3', 'dav', 'json', 'processor')),
    created_at TIMESTAMPTZ NOT NULL,

    UNIQUE(object_id, id),
    UNIQUE(object_id, generation),
    UNIQUE(object_id, external_version_id),
    FOREIGN KEY(object_id, application_id, bucket_id)
        REFERENCES objects(id, application_id, bucket_id),
    CHECK (
        (retention_mode IS NULL AND retain_until_at IS NULL)
        OR
        (retention_mode IS NOT NULL AND retain_until_at IS NOT NULL)
    ),
    CHECK (
        (
            version_kind = 'delete_marker'
            AND storage_key IS NULL
            AND etag IS NULL
            AND size_bytes IS NULL
        )
        OR
        (
            version_kind = 'data'
            AND storage_key IS NOT NULL
            AND etag IS NOT NULL
            AND size_bytes IS NOT NULL
        )
    )
);
```

`external_version_id` 的唯一范围必须是 `object_id`，不能是 `(application_id, bucket_id)`。不同 Key 都可以合法拥有 `versionId=null`；同一个 Object 最多只有一个 null version：

```sql
CREATE UNIQUE INDEX object_versions_one_null_idx
ON object_versions(object_id)
WHERE external_version_id = 'null';
```

`external_version_id = 'null'` 就是 null version，不再额外保存 `is_null_version`，避免两个字段不一致。`version_kind` 代替 `is_delete_marker`。`is_latest` 不落库。

`objects.current_version_id` 使用同 Object 复合外键，避免指向另一个 Object 的版本：

```sql
ALTER TABLE objects
ADD CONSTRAINT objects_current_version_fkey
FOREIGN KEY(id, current_version_id)
REFERENCES object_versions(object_id, id)
DEFERRABLE INITIALLY DEFERRED;
```

索引：

```sql
CREATE INDEX object_versions_history_idx
ON object_versions(object_id, generation DESC);

CREATE INDEX object_versions_external_idx
ON object_versions(object_id, external_version_id);

CREATE INDEX object_versions_noncurrent_idx
ON object_versions(bucket_id, became_noncurrent_at)
WHERE became_noncurrent_at IS NOT NULL;
```

`became_noncurrent_at` 是 `NoncurrentVersionExpiration` 的时间基准。创建新 head 时必须在同一事务内更新旧 head 的该字段；旧版本重新成为 head 时必须清空。

### 4.7 Upload Intent

普通 PUT、JSON 上传和 WebDAV PUT 共用 Upload Intent。它只表达尚未提交的上传，不污染对象版本历史：

- `proposed_version_id` 和确定性的 final immutable storage key；
- Application/Bucket/Key、principal 与 source protocol；
- content headers、user metadata、checksum、preconditions；
- retention/legal hold；
- expected/reserved bytes；
- temporary/final storage fence；
- pending/committing/completed/aborted 状态；
- lease token、lease expiry 与 upload expiry。

允许多个并发 PUT 指向同一 Key，不建立“同 Key 只能有一个 pending upload”的唯一索引。Blob promotion 在数据库最终事务之前完成；若最终事务失败，Upload Intent 必须保留，使 reaper 能根据确定性的 storage key 清理孤儿 blob。失败事务不能依赖在自身回滚后写 GC Task。

### 4.8 `multipart_uploads` 与 `multipart_parts`

删除当前“同 Key 只能有一个 active multipart”的唯一索引。S3 允许同 Key 存在多个 Multipart Upload。

`multipart_uploads` 必须冻结：

- upload ID、Application/Bucket/Key；
- content headers、user metadata、tags；
- checksum algorithm；
- Object Lock retention/legal hold；
- owner principal；
- proposed version ID 与 immutable final storage key；
- state、completion lease、expiry；
- 完成后的 committed version ID、ETag 与响应重放数据。

`multipart_parts` 保存 part number、size、真实 MD5/ETag、checksum 集合、storage key、provider fence 与更新时间。同 part number 重传后，旧 part 进入 GC。

Multipart Upload 和 Part 在 Complete 前都不进入 `object_versions`。Complete 的最终事务必须一次性完成 Multipart 终态、Object Version、Current Head、Quota 和 Outbox，详见第 11 章。

### 4.9 `storage_gc_tasks`

物理删除与逻辑删除解耦，但删除任务必须持久化：

```sql
CREATE TABLE storage_gc_tasks (
    id UUID PRIMARY KEY,
    application_id UUID NOT NULL,
    storage_backend TEXT NOT NULL,
    storage_key TEXT NOT NULL,
    size_bytes BIGINT NOT NULL DEFAULT 0,
    reason TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    available_at TIMESTAMPTZ NOT NULL,
    lease_token UUID,
    leased_until TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE(storage_backend, storage_key),
    CHECK ((lease_token IS NULL) = (leased_until IS NULL))
);
```

删除 Object Version 时，在同一事务内重新检查 Retention/Legal Hold，移除逻辑历史并写入原始 blob、Variant blob 的 GC Task，同时更新 Quota 和 Outbox。事务提交后 GC Worker 才能幂等删除 blob。Worker 使用 lease 与 `FOR UPDATE SKIP LOCKED`，不得根据过期的预扫描结果直接物理删除。

默认 GC 宽限期为 24 小时，通过配置覆盖；宽限期不改变逻辑删除的即时可见性。

### 4.10 依赖版本身份的表

| 当前依赖 | 最终修改 |
|---|---|
| `variants.media_id` | `source_version_id REFERENCES object_versions(id)` |
| `short_links.media_id` | `object_version_id`，保证短链内容稳定 |
| `upload_sessions.media_id` | `proposed_version_id`，提交后记录 `object_version_id` |
| `async_job_item_results.media_id` | `object_id` + optional `object_version_id` |
| `s3_multipart_uploads.media_id` | `committed_version_id` |
| Outbox payload 的 `media_id` | `object_id/version_id/bucket/key` |

Variant 唯一约束：

```sql
UNIQUE(source_version_id, transform_key, processor_version)
```

只有这些消费者全部切换后，才能删除 `media` 表和 `MediaId`。

### 4.11 DAV 表

```sql
CREATE TABLE dav_collections (
    application_id UUID NOT NULL,
    bucket_id UUID NOT NULL,
    path TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY(application_id, bucket_id, path)
);

CREATE TABLE dav_locks (
    token UUID PRIMARY KEY,
    application_id UUID NOT NULL,
    bucket_id UUID NOT NULL,
    path TEXT NOT NULL,
    depth TEXT NOT NULL CHECK (depth IN ('0', 'infinity')),
    owner_xml TEXT,
    fence_generation BIGINT,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX dav_locks_path_idx
ON dav_locks(application_id, bucket_id, path, expires_at);

CREATE TABLE dav_dead_properties (
    application_id UUID NOT NULL,
    bucket_id UUID NOT NULL,
    path TEXT NOT NULL,
    namespace TEXT NOT NULL,
    property_name TEXT NOT NULL,
    property_xml TEXT NOT NULL,
    PRIMARY KEY(application_id, bucket_id, path, namespace, property_name)
);
```
## 5. `mediahub-core` 修改

### 5.1 开发期新增、最终删除

先新增并验证：

```text
mediahub-core/src/
  object.rs
  object_key.rs
  object_version.rs
  entity_tag.rs
  checksum.rs
  versioning.rs
  object_metadata.rs
  object_condition.rs
  object_lock.rs
  lifecycle/
    mod.rs
    configuration.rs
    rule.rs
    filter.rs
    evaluator.rs
```

在 `ids.rs` 增加 `ObjectId` 和 `ObjectVersionId`。开发阶段旧 `media.rs` 可以继续为尚未迁移的消费者编译，但新 S3 纵向切片不得再写入旧 `Media`。只有第 20 章的消费者切换门禁全部通过后，才删除：

- `src/media.rs`、`Media`、`NewMedia`、`PersistedMedia` 与 `MediaState`；
- 旧 `MediaId` 导出；
- 旧 `BucketPolicy` 与自定义 lifecycle；
- mutable content/metadata revision 更新逻辑。

### 5.2 核心类型

```rust
pub struct Object {
    id: ObjectId,
    application_id: ApplicationId,
    bucket_id: BucketId,
    key: ObjectKey,
    current_version_id: Option<ObjectVersionId>,
    generation: u64,
}

pub struct ObjectVersion {
    id: ObjectVersionId,
    object_id: ObjectId,
    external_version_id: S3VersionId,
    generation: u64,
    kind: ObjectVersionKind,
    etag: Option<EntityTag>,
    checksums: ChecksumSet,
    metadata: ObjectMetadata,
    storage: Option<StoredObjectRef>,
    became_noncurrent_at: Option<OffsetDateTime>,
    retention: Option<ObjectRetention>,
    legal_hold: bool,
    created_at: OffsetDateTime,
}

pub enum ObjectVersionKind {
    Data,
    DeleteMarker,
}

pub enum S3VersionId {
    Null,
    Opaque(String),
}
```

`ObjectVersion` 一经提交，内容、ETag、checksum、metadata 与 storage reference 不可变。Retention、Legal Hold 通过专用命令更新并递增 `lock_revision`。Upload Intent 和 Multipart 是独立状态机，不提供“创建 staged ObjectVersion”的 API。

### 5.3 Versioning

```rust
pub enum VersioningStatus {
    Unversioned,
    Enabled,
    Suspended,
}
```

状态与行为：

- 新 Bucket：`Unversioned`；
- `Unversioned -> Enabled -> Suspended -> Enabled`；
- 一旦 Enabled，不可回到 Unversioned；
- Object Lock 启用时强制 Enabled，之后不可关闭且不可 Suspended；
- Unversioned PUT：创建/替换该 Object 的 `null` data version；
- Enabled PUT：创建 opaque version ID；
- Suspended PUT：创建/替换 `null` data version，保留 opaque 历史；
- Enabled simple DELETE：创建 opaque delete marker；
- Suspended simple DELETE：移除已有 null data version，再创建 `versionId=null` delete marker；
- exact version DELETE：永久删除指定 data version 或 delete marker；
- 删除 current version/delete marker 后，下一个最新版本成为 head；
- ListObjects 排除 current delete marker；
- ListObjectVersions 返回版本和 delete marker，并根据 head 指针计算 `IsLatest`。

### 5.4 删除与 Object Lock 领域类型

```rust
enum DeleteTarget {
    Current,
    Exact(S3VersionId),
}

enum DeleteOutcome {
    NoOp,
    PermanentlyDeleted {
        version_id: S3VersionId,
        was_delete_marker: bool,
    },
    DeleteMarkerCreated {
        version_id: S3VersionId,
    },
}
```

Object Lock 判定必须是纯领域函数，输入当前时间、Retention Mode、Retain Until、Legal Hold、bypass header 与 `s3:BypassGovernanceRetention` 权限，输出允许或拒绝。规则不能散落在 HTTP handler、Repository 和 GC Worker：

- Legal Hold ON：拒绝永久删除；
- Compliance 未到期：始终拒绝，且只允许延长日期；
- Governance 未到期：必须同时提供 bypass header 并拥有 bypass action；
- simple DELETE 创建 delete marker 不受普通版本 retention 阻止；
- delete marker 不继承 data version 的 retention。

### 5.5 ETag 与 checksum

删除 `etag = sha256` 的旧语义：

- `EntityTag` 为独立客户端可见类型；
- 普通单段 PUT 默认生成内容 MD5 风格 ETag；
- Multipart ETag 按各 Part MD5 串联后计算 `digest-partCount`；
- SHA-256 用于完整性、内部内容身份与 Variant；
- provider ETag/version 只存在于 Storage Adapter；
- 条件请求只比较 client-facing ETag；
- checksums 按算法独立保存。

## 6. `mediahub-app` 修改与事务边界

### 6.1 服务与 Repository 边界

新增：

```text
mediahub-app/src/
  object_service.rs
  object_repository.rs
  object_commands.rs
  object_queries.rs
  bucket_service.rs
  bucket_configuration_repository.rs
  upload_intent_repository.rs
  storage_gc_repository.rs
  lifecycle_service.rs
  dav_service.rs
```

`BucketConfigurationRepository`：

```text
get_versioning
put_versioning
get_lifecycle
put_lifecycle
delete_lifecycle
get_object_lock_configuration
put_object_lock_configuration
```

`put_versioning`/`put_object_lock_configuration` 自己拥有事务，并保证 Versioning 状态机、Object Lock 自动启用 Versioning、不可关闭和禁止 Suspended。

`ObjectRepository`：

```text
find_head
find_version
list_current
list_versions
commit_put
commit_copy
commit_multipart
delete_object
put_tags
delete_tags
put_retention
put_legal_hold
apply_lifecycle_action
```

Repository mutation 不暴露 SQL Transaction 给上层。每个 mutation 由 PostgreSQL Adapter 自己开启、锁定并提交事务，以保证版本、head、quota、outbox 和 GC Task 不被拆开。

`UploadIntentRepository`：

```text
create_upload_intent
claim_upload_intent
commit_upload_intent
abort_upload_intent
expire_upload_intents
```

`StorageGcRepository`：

```text
claim_gc_tasks
mark_gc_completed
release_gc_task
```

### 6.2 统一锁顺序

所有对象 mutation 使用同一锁顺序：

1. Bucket；
2. Object Key；
3. Upload Intent 或 Multipart Upload；
4. Application quota。

禁止一条路径先锁 quota、另一条路径先锁 Object。Repository Contract 必须包含死锁重试和高并发热点 Key 场景。

### 6.3 PutObject 纵向流程

事务外：

1. 解析 headers、metadata、checksum、preconditions 与 Object Lock headers；
2. 创建 Upload Intent 并预留 quota；
3. 流式写入 temporary blob；
4. 校验长度、Content-MD5 与 checksum；
5. 提升到 `objects/{proposed_version_id}` 形式的不可变 storage key。

`commit_put` 事务内：

1. `SELECT bucket ... FOR SHARE`；
2. `INSERT objects ... ON CONFLICT DO NOTHING`；
3. `SELECT objects ... FOR UPDATE`；
4. 锁定 Upload Intent；
5. 在事务内检查 `If-Match` / `If-None-Match`，避免 TOCTOU；
6. 根据 Bucket Versioning 计算 opaque 或 null version ID；
7. 对 Unversioned/Suspended 处理旧 null version，并为旧 blob/Variant 写 GC Task；
8. 将旧 head 的 `became_noncurrent_at` 设置为当前时间；
9. 插入新的 committed `object_versions`；
10. 更新 `objects.current_version_id/generation`；
11. 把 reserved quota 转为 used quota；
12. 写 Outbox；
13. 标记 Upload Intent completed；
14. 提交。

无条件并发 PUT 同 Key 时，所有 Enabled 版本都保留，最后提交事务的版本成为 head。事务失败时 Upload Intent 保留，由 reaper 清理已经 promotion 的孤儿 blob；不能假设失败事务还能写入 GC Task。

### 6.4 CopyObject

执行顺序：

1. 授权来源 `s3:GetObject` 与目标 `s3:PutObject`；
2. 固定 source bucket/key/version 和 ETag；
3. 校验 source conditions；
4. 生成目标 metadata/tags/checksum；
5. 优先使用 `ObjectStore.copy`，不支持时 stream copy；
6. 通过 `commit_copy` 创建目标新版本并原子更新 head/quota/outbox；
7. 发送 `s3:ObjectCreated:Copy`。

### 6.5 DeleteObject 与持久化 GC

`delete_object` 必须在一个数据库事务内：

1. 锁 Bucket 与 Object；
2. 解析 Current 或 Exact target；
3. exact delete 重新读取 `lock_revision` 并检查 Retention/Legal Hold；
4. 按 Unversioned/Enabled/Suspended 语义永久移除版本或创建 delete marker；
5. 更新旧/新 head 的 `became_noncurrent_at` 与 Object generation；
6. 为被移除 data version 的原始 blob 和所有 Variant blob 写 `storage_gc_tasks`；
7. 更新 Quota；
8. 写 Outbox；
9. 提交。

事务提交后 GC Worker 才能按 `available_at`、lease 和 storage fence 幂等删除 blob。删除前不得使用“扫描时检查过锁，所以现在直接删”的逻辑；Object Lock 必须在逻辑删除事务内重检。不存在 Key 的 simple DELETE 返回 204。

### 6.6 强类型 Command

不要使用巨型 options bag。分别定义 `PutObjectCommand`、`GetObjectQuery`、`HeadObjectQuery`、`CopyObjectCommand`、`DeleteObjectCommand`、`DeleteObjectsCommand`、`ListObjectsQuery`、`ListObjectVersionsQuery`、`CompleteMultipartCommand`、`PutRetentionCommand` 与 `PutLegalHoldCommand`。

每个 Command 携带 principal、Application/Bucket/Key、version、preconditions 和 operation-specific metadata。领域层不得依赖 Axum `HeaderMap`、`Uri` 或 HTTP method。
## 7. `mediahub-adapter-postgres` 修改

### 7.1 开发阶段与最终清理

先新增并通过 Contract 验证：

```text
mediahub-adapter-postgres/src/
  object_repository.rs
  object_queries.rs
  object_commands.rs
  object_versions.rs
  bucket_repository.rs
  bucket_configuration.rs
  upload_intent_repository.rs
  multipart_repository.rs
  lifecycle_repository.rs
  storage_gc_repository.rs
  dav_repository.rs
```

旧 `media.rs`、`media_buckets.rs`、`media_queries.rs`、`media_mutations.rs` 与 `media_support.rs` 在消费者尚未切换时保留。新 S3 闭环不能调用它们；所有消费者切换后再整体删除，把仍有用的 Bucket 查询迁入 `bucket_repository.rs`。

### 7.2 当前对象与版本列表

ListObjects：

- 从 `objects` join `object_versions current`；
- 排除 current delete marker；
- 按原始 UTF-8 object key 的稳定字节序排序；
- prefix 使用范围条件或合适索引；
- delimiter 稳定生成 CommonPrefixes；
- continuation token 通过 HMAC 绑定 Application、Bucket、prefix、delimiter 和 key cursor。

ListObjectVersions：

1. object key ascending；
2. generation descending；
3. 使用 key-marker + version-id-marker 分页；
4. 返回 Version 与 DeleteMarker 混合结果；
5. `IsLatest` 由 `objects.current_version_id` 比较得出，不读取冗余列。

### 7.3 并发与事务实现

- `INSERT ... ON CONFLICT DO NOTHING` 创建 Object；
- `SELECT ... FOR UPDATE` 锁单 Key；
- Bucket/Object/Intent-or-Multipart/Quota 使用固定锁顺序；
- mutation 在 Adapter 内开启事务，不把 transaction handle 暴露给应用层；
- GC Worker 使用 `FOR UPDATE SKIP LOCKED` claim task；
- lifecycle action 通过 generation、configuration revision、lock revision 防止使用陈旧扫描结果；
- 优先保证单 Key 强一致，再压测热点 Key 和分页查询。

### 7.4 必须提供的故障注入点

Repository Contract 至少能够模拟：

- 插入 Object Version 后、更新 head 前失败；
- 更新 head 后、写 Quota/Outbox 前失败；
- Multipart Complete 最终事务提交失败；
- GC Task claim 后 storage delete 失败；
- Retention/Legal Hold 在 Lifecycle 扫描后、执行前变化；
- Outbox 插入失败。

这些故障都不得留下“版本可读但状态未完成”“blob 已物理删除但版本仍存在”或 quota 漂移。
## 8. `mediahub-server` 的 S3 模块

### 8.1 新目录

```text
crates/mediahub-server/src/s3/
  mod.rs
  router.rs
  operation.rs
  operation_context.rs
  auth.rs
  error.rs
  response.rs
  headers.rs
  conditions.rs
  checksum.rs
  handlers/
    mod.rs
    service.rs
    bucket.rs
    object.rs
    multipart.rs
    versioning.rs
    tagging.rs
    policy.rs
    lifecycle.rs
    notification.rs
    object_lock.rs
  xml/
    mod.rs
    list.rs
    versions.rs
    delete.rs
    multipart.rs
    policy.rs
    lifecycle.rs
    object_lock.rs
  auth/
    sigv4.rs
    streaming.rs
```

### 8.2 Operation classifier

Axum 不能像 Silo 的 mux 一样直接按 query 注册全部 handler，因此实现一个纯函数：

```rust
fn classify(request: &S3RequestParts) -> Result<S3Operation, S3Error>
```

输入：

- method；
- root/bucket/object path；
- query key/value；
- `x-amz-copy-source`；
- `Content-Type`；
- 必要 Header。

输出 enum：

```rust
enum S3Operation {
    ListBuckets,
    CreateBucket,
    HeadBucket,
    DeleteBucket,
    GetBucketLocation,
    ListObjectsV1,
    ListObjectsV2,
    ListObjectVersions,
    GetBucketVersioning,
    PutBucketVersioning,
    GetBucketPolicy,
    PutBucketPolicy,
    DeleteBucketPolicy,
    GetBucketLifecycle,
    PutBucketLifecycle,
    DeleteBucketLifecycle,
    GetBucketObjectLockConfiguration,
    PutBucketObjectLockConfiguration,
    GetBucketNotification,
    PutBucketNotification,
    PutObject,
    CopyObject,
    GetObject,
    HeadObject,
    DeleteObject,
    DeleteObjects,
    GetObjectTagging,
    PutObjectTagging,
    DeleteObjectTagging,
    CreateMultipartUpload,
    UploadPart,
    UploadPartCopy,
    ListParts,
    CompleteMultipartUpload,
    AbortMultipartUpload,
    ListMultipartUploads,
    GetObjectRetention,
    PutObjectRetention,
    GetObjectLegalHold,
    PutObjectLegalHold,
}
```

classifier 只分类，不鉴权、不查数据库。

未知组合返回标准 `NotImplemented` 或 `InvalidRequest`，不能误落入 PutObject/GetObject。

### 8.3 Operation context

参考 Silo `object-api-options.go`，但使用强类型：

```rust
struct S3OperationContext {
    request_id: RequestId,
    operation: S3Operation,
    principal: Principal,
    application_id: ApplicationId,
    bucket: Option<BucketName>,
    key: Option<ObjectKey>,
    version_id: Option<S3VersionId>,
    conditions: ObjectConditions,
    checksum: RequestedChecksum,
}
```

解析顺序：

1. 解析 raw path/query；
2. classify operation；
3. 验证 SigV4；
4. 加载 Access Key/Application；
5. 生成 Resource；
6. Policy authorize；
7. handler。

### 8.4 S3 Error

独立定义：

```rust
enum S3ErrorCode {
    AccessDenied,
    NoSuchBucket,
    NoSuchKey,
    NoSuchVersion,
    BucketAlreadyExists,
    BucketNotEmpty,
    InvalidArgument,
    InvalidRequest,
    InvalidRange,
    PreconditionFailed,
    EntityTooLarge,
    BadDigest,
    InvalidDigest,
    NoSuchUpload,
    InvalidPart,
    InvalidPartOrder,
    MalformedXml,
    ObjectLockConfigurationNotFound,
    InvalidBucketState,
    OperationAborted,
    MethodNotAllowed,
    NotImplemented,
    SlowDown,
    ServiceUnavailable,
    InternalError,
}
```

`S3Error` 保存：

- code；
- HTTP status；
- resource；
- request ID；
- optional headers；
- internal cause（不输出）。

所有 S3 response 都设置：

- `x-amz-request-id`
- 可选 `x-amz-id-2`

不要通过 `ApiError` 生成 S3 XML。

### 8.5 Handler 校验顺序

Handler 只编排协议步骤，状态语义和事务一致性由 Service/Repository 保证。

PutObject：

1. operation classifier；
2. path/key/query 校验；
3. SigV4；
4. 加载 Access Key/Application；
5. 解析 size、metadata、checksum、conditions 与 Object Lock headers；
6. 查找 Bucket；
7. 授权 `s3:PutObject`；
8. 校验 Bucket 限制、Versioning 与 Object Lock 配置；
9. streaming write + checksum verification；
10. immutable promotion；
11. `ObjectService::commit_put`；
12. 返回 ETag、version ID 和 checksum headers。

Get/Head：

1. 解析 `versionId`；
2. SigV4 与授权；
3. 查找 Bucket；
4. 解析 Range/conditions；
5. resolve current/exact version；
6. 当前 head 是 delete marker：返回 `404 NoSuchKey` 和 `x-amz-delete-marker: true`；
7. 显式读取 delete marker：返回 `405 MethodNotAllowed`、delete marker 和 version headers；
8. 返回版本 metadata、condition/range headers 与 body。

DeleteObject：

1. 解析 `versionId` 和 governance bypass header；
2. SigV4；
3. simple delete 授权 `s3:DeleteObject`，exact delete 授权 `s3:DeleteObjectVersion`；
4. 请求 bypass 时额外授权 `s3:BypassGovernanceRetention`；
5. 调用事务型 `ObjectService::delete_object`；
6. 返回 204、`x-amz-version-id` 与必要的 `x-amz-delete-marker`。

PutLifecycle：

1. SigV4；
2. 授权 `s3:PutLifecycleConfiguration`；
3. 校验 Content-MD5 与 XML；
4. 转换为领域配置；
5. 显式拒绝未支持的 Filter/Action；
6. 事务更新 configuration revision；
7. 返回 200。

PutBucketObjectLockConfiguration：

1. SigV4；
2. 授权 `s3:PutBucketObjectLockConfiguration`；
3. 校验 Content-MD5/XML；
4. 事务锁定 Bucket；
5. 首次启用时同步设置 Versioning=Enabled；
6. 禁止关闭 Object Lock；
7. 更新默认 Retention；
8. 返回 200。
## 9. 第一阶段 S3 纵向闭环

第一阶段目标不是一次完成所有 S3 功能，而是先让 Versioning、Delete Marker、Multipart、Object Lock 与有限 Lifecycle 建立在正确版本模型上。独立 listener 已从该纵向闭环开始启用，标准入口直接使用 `/{bucket}` 与 `/{bucket}/{key}`；旧 `/s3` endpoint 不再保留。

必须完成：

### Bucket

- ListBuckets、CreateBucket、HeadBucket、DeleteBucket、GetBucketLocation；
- ListObjectsV1/V2、ListObjectVersions；
- Get/PutBucketVersioning；
- Get/Put/DeleteBucketLifecycleConfiguration；
- Get/PutBucketObjectLockConfiguration；
- ListMultipartUploads。

### Object

- PutObject、GetObject、HeadObject；
- DeleteObject、DeleteObjects，支持 simple delete、Delete Marker 与 exact `versionId`；
- Get/PutObjectRetention；
- Get/PutObjectLegalHold；
- Object Lock headers on PutObject。

### Multipart

- CreateMultipartUpload、UploadPart、ListParts；
- CompleteMultipartUpload、AbortMultipartUpload；
- Initiate 阶段冻结 metadata、checksum 与 Object Lock headers；
- Complete 创建 Object Version 并原子提交 head/quota/outbox/multipart 终态。

### Lifecycle 第一阶段子集

只接受：

- 空 Filter 或 Prefix Filter；
- Expiration；
- NoncurrentVersionExpiration；
- ExpiredObjectDeleteMarker；
- AbortIncompleteMultipartUpload。

第一阶段明确拒绝 Tag、And、ObjectSize、Transition、NoncurrentVersionTransition 和其他未实现动作，返回标准 S3 错误；禁止“配置保存成功但 Worker 静默忽略”。

### 请求行为

- SigV4 Header 与 Presigned；
- Content-MD5、SHA-256 checksum；
- Range；
- If-Match、If-None-Match、If-Modified-Since、If-Unmodified-Since；
- versionId/null version/delete marker response headers；
- 标准 S3 XML error。

CopyObject/UploadPartCopy 与版本级 Object Tagging 已由后续纵向切片完成。仍待实现的是 Bucket Policy、Notification、Bucket Tagging、CORS、POST Policy、SSE-S3、GetObjectAttributes、更多 checksum 与临时 session token；未实现 operation 必须返回标准错误，不能假成功。
## 10. Policy 实现

### 10.1 Core 类型

```rust
enum S3Action {
    ListAllMyBuckets,
    CreateBucket,
    DeleteBucket,
    ListBucket,
    ListBucketVersions,
    GetBucketLocation,
    GetBucketVersioning,
    PutBucketVersioning,
    GetBucketPolicy,
    PutBucketPolicy,
    GetObject,
    PutObject,
    DeleteObject,
    DeleteObjectVersion,
    GetObjectTagging,
    PutObjectTagging,
    AbortMultipartUpload,
    BypassGovernanceRetention,
    // ...
}
```

Resource：

- `arn:aws:s3:::bucket`
- `arn:aws:s3:::bucket/key`

Policy 支持：

- Version；
- Statement；
- Effect Allow/Deny；
- Action/NotAction；
- Resource/NotResource；
- Principal（Bucket Policy）；
- Condition 第一批子集。

第一批 Condition：

- `s3:prefix`
- `s3:delimiter`
- `s3:ExistingObjectTag/*`
- `s3:RequestObjectTag/*`
- `s3:VersionId`
- `aws:SecureTransport`
- source IP

求值：

1. Application boundary；
2. explicit deny；
3. identity policy allow；
4. bucket policy allow；
5. default deny。

没有 legacy permissions fallback。

### 10.2 WebDAV 权限映射

| DAV | S3 Action |
|---|---|
| PROPFIND bucket | ListBucket |
| GET/HEAD | GetObject |
| PUT | PutObject |
| DELETE | DeleteObject / DeleteObjectVersion |
| COPY source | GetObject |
| COPY target | PutObject |
| MOVE | GetObject + PutObject + DeleteObject |
| MKCOL | PutObject-compatible DAV collection action |
| LOCK/UNLOCK | MediaHub DAV lock action |

DAV lock action可以是 MediaHub 扩展，但对象内容权限仍由 S3 Action 控制。

## 11. Multipart 修改

### 11.1 删除当前限制

最终删除：

- 同 Key 唯一 active upload 索引；
- Initiate 只保存 content type/visibility 的旧模型；
- Complete 通过旧 `UploadMediaService` 创建 Media 的路径；
- Abort 使用 `media:upload` 权限的逻辑。

这些删除发生在 Multipart 已接入 Object/ObjectVersion 纵向闭环之后，不能先删除旧路径再等待新 Versioning 模型补齐。

### 11.2 Initiate 与 UploadPart

CreateMultipartUpload：

- 授权 `s3:PutObject`；
- 冻结 content headers、user metadata、tags、checksum algorithm；
- 冻结 retention mode/date、legal hold 与 owner principal；
- 分配 upload ID、proposed version ID 和 immutable final storage key；
- Object Lock Bucket 在锁相关 headers 尚未完整支持前必须拒绝 Initiate，不能降级忽略。

UploadPart：

- 同 part number 重传替换旧 part，并为旧 part 写 GC Task；
- 保存真实 MD5、client-facing ETag、checksum 和 provider fence；
- part number 范围 1..10000；
- 非最后 part 的最小 5 MiB 在 Complete 时校验。

### 11.3 Complete 原子边界

事务外：

1. claim completion lease；
2. 严格解析并验证有序 Part 列表；
3. 校验每个 Part ETag；
4. compose temporary blob；
5. 计算 Multipart ETag 与请求的 checksum；
6. promotion 到 proposed version 的不可变 storage key。

最终 PostgreSQL 事务必须一次性完成：

1. 锁定 Bucket、Object、Multipart Upload 和 Quota；
2. 重新确认 Multipart 仍为 `completing` 且 lease 有效；
3. 在事务内检查目标条件与 Bucket Versioning/Object Lock 状态；
4. 处理旧 null version 与旧 head 的 `became_noncurrent_at`；
5. 插入 committed Object Version；
6. 更新 Current Head/generation；
7. 将 reserved quota 转为 used quota；
8. 写 Outbox；
9. 把 Multipart 状态更新为 `completed`，持久化 version ID、ETag 和可重放响应；
10. 提交。

禁止把 `commit Object Version` 与 `finish multipart completion` 拆成两个事务。事务提交失败时 Multipart 保持可恢复状态，已 promotion 的 blob 由 Upload/Multipart reaper 清理。Complete 重放返回同一 version ID/ETag，不重复创建版本或事件。

### 11.4 Abort 与 List

Abort：

- 授权 `s3:AbortMultipartUpload`；
- 与 Complete 竞争同一状态机和 lease；
- 幂等进入 `aborted`；
- 在事务中释放 reservation、为所有 Part/临时 blob 写 GC Task；
- 已完成的 Upload 不得再被 Abort 回滚。

ListMultipartUploads 支持 prefix、delimiter、key-marker、upload-id-marker 和 max-uploads，数据源是 PostgreSQL，而不是 storage backend list。

## 12. Lifecycle 与 Object Lock

### 12.1 Lifecycle 第一阶段能力

```text
mediahub-core/src/lifecycle/
  configuration.rs
  rule.rs
  filter.rs
  evaluator.rs
```

Parser、validator 与 evaluator 分离。第一阶段只接受：

- 空 Filter 或 Prefix Filter；
- Expiration；
- NoncurrentVersionExpiration；
- ExpiredObjectDeleteMarker；
- AbortIncompleteMultipartUpload。

Tag、And、ObjectSize、Transition、NoncurrentVersionTransition 及其他未支持条件/动作在 PUT 配置时明确拒绝，不能保存后静默不执行。

Evaluator 输入 current/noncurrent、version kind、created time、`became_noncurrent_at`、retention/legal hold、multipart created time 与当前配置 revision，输出：

- `ExpireCurrent`
- `ExpireNoncurrentVersion`
- `DeleteExpiredDeleteMarker`
- `AbortMultipart`
- `None`

行为：

- Current Expiration：Unversioned 永久删除 null data version；Enabled 创建 opaque delete marker；Suspended 创建/替换 null delete marker；
- NoncurrentVersionExpiration：永久删除指定非当前版本；
- ExpiredObjectDeleteMarker：仅当 marker 是唯一剩余版本时删除；
- AbortIncompleteMultipartUpload：终止 Upload、释放 reservation，并为 Part 写 GC Task；
- 被 Object Lock 保护的非当前版本不能被 Lifecycle 永久删除，但 Current Expiration 仍可创建 delete marker。

Worker 可以重复扫描，但执行动作必须通过 `apply_lifecycle_action` 在事务内重新检查 configuration revision、Object generation、current/noncurrent 状态、Retention、Legal Hold 与 Multipart 终态。Worker 不能直接物理删除 blob。

### 12.2 Object Lock 配置状态

- Object Lock 可以在 Bucket 创建时或创建后启用；
- 首次启用时，在同一事务内把 Versioning 设置为 Enabled；
- 一旦启用不可关闭；
- 启用后禁止 Versioning Suspended；
- Bucket 可以启用 Object Lock 但不配置 Default Retention；
- 默认 Retention 只应用于之后创建的新 data version；
- retention 和 legal hold 始终绑定具体 Object Version。

### 12.3 永久删除顺序

任何用户删除、Lifecycle 删除和后台清理都遵循：

1. 在数据库事务内锁定 Bucket/Object/Version；
2. 重新读取 `lock_revision`、Retention 与 Legal Hold；
3. 检查 Compliance、Governance 和 bypass 权限；
4. 从逻辑版本历史中移除目标版本并重算 head；
5. 同事务写入 GC Task、Quota 变化和 Outbox；
6. 提交；
7. GC Worker 在默认 24 小时宽限期后幂等删除 blob。

不再采用“先标记 deleting，Worker 之后先删 blob，再回数据库 finalize”的顺序。GC Worker 只执行持久化任务，不自行解释 Object Lock 或 Lifecycle 规则。
## 13. WebDAV 重写

### 13.1 保留

- `dav-server`；
- Basic Auth 入口；
- `/dav` 路由；
- 目录由 Key prefix 投影的产品体验。

### 13.2 重写文件

```text
mediahub-server/src/webdav/
  mod.rs
  auth.rs
  filesystem.rs
  file.rs
  locks.rs
  properties.rs
  path.rs
```

旧 `webdav_auth.rs`、`webdav_fs.rs`、`webdav_file.rs`、`webdav_support.rs` 删除。

### 13.3 统一行为

- GET/HEAD -> ObjectService get/head；
- PUT -> ObjectService put；
- DELETE -> ObjectService delete；
- COPY -> ObjectService copy；
- MOVE -> copy + 条件 delete；
- PROPFIND -> ObjectService list/head；
- MKCOL -> `dav_collections`；
- LOCK/UNLOCK -> `dav_locks`；
- PROPPATCH -> `dav_dead_properties`。

ETag 必须与 S3 返回相同值。

PUT、DELETE、COPY、MOVE 都检查：

- DAV lock token；
- S3 Policy；
- HTTP/DAV conditions。

MOVE 不承诺跨 Key 原子。实现：

1. 固定来源 version/ETag；
2. Copy；
3. If-Match delete source；
4. 失败写 operation result，客户端可重试。

## 14. 预览修改

### 14.1 不重写 Viewer

保留：

- `ObjectFileViewer.tsx`
- Open File Viewer plugins；
- archive/spreadsheet/SQLite plugins；
- Web Worker、timeout、大小限制和 SVG 降权。

### 14.2 修改读取身份

当前 preview URL 如果只跟 `media_id` 或当前 Key 绑定，统一改为：

```text
object_version_id
```

新增：

```text
GET /api/v1/object-versions/{version_id}/preview-manifest
GET /api/v1/object-versions/{version_id}/content
```

Preview Manifest 返回：

- version ID；
- ETag；
- content type；
- size；
- renderer；
- renderer version；
- mode（stream/buffered）；
- max bytes；
- signed version content URL；
- expiry；
- warnings。

`/api/v1/capabilities` 返回实际 renderer 数组，不再只有 `image_processing: true` 这类布尔值。

### 14.3 缓存

缓存键：

```text
source_version_id
+ source_sha256
+ renderer
+ renderer_version
+ normalized_options
```

同 Key 覆盖后旧预览不变化，新版本生成自己的预览结果。

## 15. 图片 Variant 修改

### 15.1 保留

- transform normalization；
- SHA-256；
- processor version；
- claim/lease/reclaim/fencing；
- ready/failed；
- temporary -> commit。

### 15.2 修改

`GenerateVariantRequest` 删除：

- `media_sha256` 作为唯一来源身份；
- 可变 Media 对象依赖。

改为：

```rust
pub struct GenerateVariantRequest {
    pub source_version_id: ObjectVersionId,
    pub source_sha256: String,
    pub transform: ImageTransform,
}
```

cache key：

```text
source_version_id + source_sha256 + normalized_transform + processor_version
```

API：

- `POST /api/v1/object-versions/{version_id}/variants`
- `GET /api/v1/variants/{variant_id}`
- `GET /api/v1/variants/{variant_id}/content`

Variant 不进入 S3 ListObjects。

如果用户要把 Variant 变成正式对象：

- 调用 materialize；
- 目标通过 ObjectService Put/Copy 创建普通对象版本。

### 15.3 视频

本轮不建视频处理服务、不加队列、不加 FFmpeg。

只要求：

- Variant/derived output schema 不把 processor 写死为 image；
- `processor_kind` 可扩展；
- ObjectService 支持 processor 以受限 principal 写正式输出。

## 16. Storage Adapter 修改

### 16.1 保留

当前 `ObjectStore` 的：

- put temporary；
- compose temporary；
- commit temporary；
- read/range/head；
- list/delete；
- checksum；
- provider ETag/version fencing。

### 16.2 新增

```rust
async fn copy(
    &self,
    source_key: &str,
    source_fence: &ProviderFence,
    target_key: &str,
) -> Result<ObjectMetadata, ObjectStoreError>;

async fn delete_if_match(
    &self,
    key: &str,
    fence: &ProviderFence,
) -> Result<(), ObjectStoreError>;
```

`ProviderFence`：

- provider ETag；
- provider version。

### 16.3 不允许

- ObjectService 使用 provider version 作为 S3 version ID；
- 把 provider ETag 直接返回客户端；
- storage backend list 作为 S3 ListObjects 数据源；
- final storage key 等于用户 object key。

## 17. OpenAPI 与 Web 修改

### 17.1 OpenAPI 删除

- Media DTO；
- fixed Permission enum；
- old lifecycle DTO；
- old mutable metadata revision；
- 旧 Variant media_id request。

### 17.2 OpenAPI 新增

- ObjectSummary；
- ObjectVersion；
- VersioningStatus；
- ObjectTag；
- PreviewManifest；
- AccessKeyPolicyDocument；
- LifecycleConfiguration；
- Variant source version；
- Object history list。

S3 XML API 不放进 OpenAPI。

### 17.3 Web 页面

对象列表：

- 展示当前 head；
- delete marker 不显示为普通文件；
- 可进入版本历史。

对象详情：

- 当前版本；
- 历史版本；
- delete marker；
- ETag/checksum；
- retention/legal hold；
- tags；
- Preview；
- Variants。

Bucket 设置：

- Versioning；
- Lifecycle；
- Bucket Policy；
- Object Lock；
- Notification。

Access Key：

- policy template；
- JSON editor；
- policy validation；
- policy simulator；
- 不再显示固定权限复选框。

## 18. 事件修改

复用当前 Outbox/Webhook。

新增标准事件：

- `s3:ObjectCreated:Put`
- `s3:ObjectCreated:Copy`
- `s3:ObjectCreated:CompleteMultipartUpload`
- `s3:ObjectRemoved:Delete`
- `s3:ObjectRemoved:DeleteMarkerCreated`
- `s3:ObjectTagging:Put`
- `s3:ObjectTagging:Delete`
- `s3:LifecycleExpiration:Delete`
- `s3:LifecycleExpiration:DeleteMarkerCreated`

保留 MediaHub 事件：

- Variant ready/failed；
- Preview render ready/failed；
- AI metadata（未来）。

Outbox 与 object/version/head/quota 在同一 DB 事务提交。

## 19. 测试策略与矩阵

### 19.1 测试结构

```text
crates/mediahub-core/tests/
  versioning.rs
  object_lock.rs
  lifecycle.rs
  entity_tag.rs

crates/mediahub-app/tests/
  object_service_put.rs
  object_service_delete.rs
  object_service_versioning.rs
  object_service_multipart.rs
  object_service_concurrency.rs

crates/mediahub-adapter-postgres/tests/
  object_repository_contract.rs
  object_repository_concurrency.rs
  bucket_configuration_contract.rs
  multipart_repository_contract.rs
  storage_gc_contract.rs
  listing_contract.rs

crates/mediahub-server/tests/s3/
  sigv4.rs
  operation_classifier.rs
  errors.rs
  bucket.rs
  object.rs
  conditions.rs
  listing.rs
  versioning.rs
  multipart.rs
  lifecycle.rs
  object_lock.rs
```

现有 S3 特征测试先保留，用于证明新旧路径在已支持行为上没有无意回退。只有替代测试覆盖相同语义后，才删除 `versionId NotImplemented`、旧 `/s3` URL、fixed permission、旧 MediaRepository 和自定义 lifecycle 测试。

### 19.2 Core 单元测试

- `Unversioned -> Enabled -> Suspended -> Enabled` 状态机；
- Enabled 不可回到 Unversioned；
- Object Lock 启用时自动 Enabled、不可关闭且禁止 Suspended；
- `S3VersionId::Null/Opaque`；
- Delete Marker 不允许 storage/etag/size；
- `is_latest` 从 current head 推导；
- Governance bypass 的 header/permission 组合；
- Compliance 只能延长、不能缩短；
- Legal Hold 与 Retention 相互独立；
- Lifecycle evaluator 表驱动测试；
- 第一阶段不支持的 Lifecycle filter/action 被 validator 拒绝。

### 19.3 PostgreSQL Repository Contract

| 场景 | 必须满足 |
|---|---|
| 100 个同 Key Enabled PUT | 保留 100 个版本，只有一个 head，generation 单调 |
| 100 个同 Key Unversioned PUT | 最终只有一个 null version，被替换 blob 均有 GC Task |
| 不同 Key 都写 null version | 全部成功，证明唯一范围是 object_id |
| 两个 `If-None-Match: *` | 仅一个成功 |
| 两个相同 `If-Match` | 仅一个成功 |
| PUT 与 DELETE 并发 | head 合法，generation 单调，无 quota 漂移 |
| 删除 current data version | 次新版本成为 head，`became_noncurrent_at` 正确更新 |
| 删除 current delete marker | 旧 data version 重新可见并清空 noncurrent 时间 |
| Suspended PUT | 替换 null，不删除 opaque versions |
| Suspended DELETE | 删除 null data，创建 null delete marker |
| Outbox 插入失败 | version/head/quota/intent 全部回滚 |
| DB commit 失败 | Upload Intent 可恢复，orphan blob 可由 reaper 清理 |
| exact delete 受锁版本 | 无逻辑删除、无 GC Task、无 quota 变化 |
| Retention 在扫描后延长 | Lifecycle 最终事务重新检查并拒绝删除 |
| Lifecycle 与用户 DELETE | 最多一次逻辑删除与 quota 变更 |
| Multipart Complete/Abort 并发 | 只能有一个终态 |
| Multipart Complete 重放 | 返回同一 version ID/ETag，不重复 Outbox |
| GC storage delete 失败 | task 可重试，逻辑状态不回滚 |

### 19.4 HTTP 集成测试

- Get/PutBucketVersioning XML 与状态转换；
- Enabled/Suspended/null version response headers；
- Get/Head 当前 delete marker；
- Get/Head 指定 delete marker；
- DeleteObject/DeleteObjects 的 simple/exact version 行为；
- ListObjectVersions 的 key-marker、version-id-marker、UTF-8 排序、IsLatest 和混合分页；
- Lifecycle PUT/GET/DELETE 与 Content-MD5；
- 未支持 Lifecycle Filter/Action 返回标准错误；
- Fake Clock 驱动 Expiration/Noncurrent/ExpiredMarker/AbortMultipart；
- Bucket Object Lock 启用、不可关闭和 Versioning 约束；
- Retention/LegalHold PUT/GET；
- Governance bypass；
- PutObject 与 Multipart 的默认 Retention/Lock headers；
- Error XML、状态码与 response headers golden tests。

### 19.5 SDK 黑盒与对照

第一道发布门禁：

- AWS CLI；
- boto3；
- AWS SDK JS v3。

扩展门禁：AWS SDK Go v2、rclone、mc。工作流覆盖 bucket、put/get/head/list/delete、overwrite、conditions、range、presigned、multipart、versioning/delete marker、lifecycle 与 object lock。

可以将同一黑盒用例同时指向 PrismArk 与本地 Silo，对照 status、headers、XML shape、error code 和可见对象状态；不比较动态 request/version ID 的具体值，也不复制其实现代码。

## 20. 主要风险与控制措施

| 风险 | 失败表现 | 控制措施 |
|---|---|---|
| 一次删除迁移和 Media 后整体重写 | 编译面、协议面和数据面同时失效，故障无法定位 | 临时开发迁移 + 纵向切片；最后才压平 0001 |
| 新旧模型长期并存或 dual-write | S3 与 DAV 看到不同对象真相 | 每个写入口切换后立即停止旧写，最终删除旧模型 |
| version ID 唯一范围错误 | 不同 Key 无法同时拥有 null version | `UNIQUE(object_id, external_version_id)` + contract test |
| 把 staged upload 写入版本历史 | List/Head 暴露半成品，失败状态污染历史 | Upload Intent/Multipart 独立状态机，提交时才插入版本 |
| Multipart Complete 拆事务 | 版本可读但 upload 仍 completing，重放重复版本 | version/head/quota/outbox/终态单事务 |
| Object Lock 预检查与物理删除分离 | 锁在扫描后变化仍被删 blob | 逻辑删除事务内重检，提交持久化 GC Task 后再物理删除 |
| Lifecycle 静默接受未支持规则 | 用户认为策略生效但数据不删除 | PUT 配置阶段显式拒绝未支持 filter/action |
| GC 只在内存排队 | 重启丢任务或重复扣 quota | PostgreSQL GC Task + lease + 幂等 storage fence |
| 过早重写 Router/Auth/Policy | 协议分类、认证、授权、版本语义故障混在一起 | 先复用当前入口完成对象纵向闭环，再独立切换 listener |
| Host/端口写死 | 反向代理、容器和虚拟主机签名失败 | bind 可配置，Host 从请求/可信代理配置导出 |

任何阶段若引入第二个对象真相源、不可重放 mutation 或无法通过故障注入测试，不得进入下一阶段。

## 21. 实施顺序

每个阶段单独提交，前一阶段验收通过再继续。开发过程允许临时迁移和旧消费者存在，但禁止新旧双写；最终仓库不保留兼容实现。

### Phase 0：现有行为特征测试

- 固化当前 Put/Get/Head/Delete/List、SigV4、Multipart 与错误 XML；
- 新增 Versioning/Delete Marker/Object Lock/Lifecycle 期望行为表；
- 建立 Fake Clock、故障注入和 SDK 测试框架。

验收：测试能区分“尚未实现”和“已有行为回退”。

### Phase 1：开发迁移与 Core 模型

- 通过临时开发迁移新增 Object/ObjectVersion、Bucket 配置、Upload Intent、Multipart 扩展与 GC Task；
- 新增 Versioning、EntityTag、Checksum、Object Lock 与 Lifecycle 类型；
- 暂不删除 Media 和旧 migrations。

验收：约束测试、Core 单测、fresh test database 通过。

### Phase 2：ObjectService 基础纵向闭环（已完成核心对象路径）

- 复用现有 SigV4 与 Application 边界，但直接使用独立 S3 listener；
- 完成 Put/Get/Head/Delete、ListObjectsV2 与 ListObjectVersions；
- 完成 null version、delete marker、head/generation、quota/outbox 和持久化 GC；
- 新 S3 路径不再写 Media。

验收：Repository Contract、并发和故障注入通过。

### Phase 3：Versioning API 与删除语义

- Get/PutBucketVersioning；
- Unversioned/Enabled/Suspended 状态机；
- exact version read/delete 与 DeleteObjects；
- 标准 version/delete marker headers 和错误 XML。

验收：AWS CLI、boto3、JS SDK v3 的版本工作流通过。

### Phase 4：Multipart 接入版本模型

- 删除同 Key active upload 限制；
- Initiate 冻结 metadata/checksum/tagging；显式 Object Lock headers 仍明确拒绝，Bucket DefaultRetention 在 Complete 事务内应用；
- Complete 原子提交 version/head/quota/outbox/终态；
- Abort 与 Complete 竞争、重放和 GC。
- ListMultipartUploads 与 UploadPartCopy 已作为独立协议切片完成。

验收：并行、重传、重放、abort/complete race 与 failure injection 通过。

### Phase 5：Object Lock（核心纵向闭环已完成）

- Bucket 创建后启用、不可关闭、自动 Enabled、禁止 Suspended；
- Default Retention、Object Retention、Legal Hold、Governance bypass；
- PutObject lock headers；Multipart 显式 lock headers 仍是后续项。

当前 PutObject、Bucket DefaultRetention、对象级 Retention/Legal Hold 已完成；CopyObject 目标锁头和 Multipart 显式锁头暂时明确拒绝，不会静默忽略。无显式锁头的 Multipart Complete 会在版本提交事务内应用 Bucket DefaultRetention。

验收：锁定版本在用户、Lifecycle 和 GC 路径均不会被提前物理删除。

### Phase 6：Lifecycle 有限子集（核心纵向闭环已完成）

- parser/validator/evaluator；
- Expiration、NoncurrentVersionExpiration、ExpiredObjectDeleteMarker、AbortIncompleteMultipartUpload；
- transaction recheck 与持久化 GC；
- 未支持 rule/action 明确拒绝。

验收：Fake Clock、并发删除、Retention 延长与配置 revision 变化测试通过。

### Phase 7：补齐 S3 扩展切片

CopyObject/UploadPartCopy 与 Object Tagging 已完成。后续依次实现 Policy、Notification，再处理 Bucket Tagging、CORS/SSE 等能力。每个切片包含 operation classifier、action、repository、HTTP golden 与 SDK 测试，不把 Router/Auth/Policy/Error 一次重写。

### Phase 8：切换其余消费者

WebDAV 普通文件路径和不可变版本 Preview 后端已经切换；后续按顺序切换 JSON API、Variant、Short Link、Batch Job、Worker 和 Web UI 的剩余路径到 Object/ObjectVersion。每切换一个消费者，就删除其旧 Media 写入口与旧契约。

验收：所有协议读取同一 head，同一版本 ETag 一致，旧预览/Variant 不因 Key 覆盖漂移。

### Phase 9：独立 S3 Listener 与 Router（核心入口已提前完成）

- 切换独立 S3 listener；
- 移除 `/s3` 前缀；
- 完成 operation classifier、S3 Error/XML、可信代理与 Host 处理；
- 默认 bind `0.0.0.0:9000`，端口可配置，公开 Host 不写死。

验收：path-style、反向代理、presigned 和 SigV4 Host 测试通过。

### Phase 10：最终清理与压平

仅在所有消费者和测试门禁通过后：

1. 删除 `Media`、`MediaRepository`、旧 S3 分支和旧 lifecycle；
2. 删除开发期临时迁移；
3. 压平为新的 `0001_initial.sql`；
4. 删除本地开发数据库/volume 并 fresh install；
5. 全仓搜索旧类型、旧路由、兼容列、dual-write 和旧 ID 映射；
6. 执行完整质量门禁。

## 22. 完成标准

### 代码与模块边界

- 不存在 `MediaRepository`、旧 mutable Media 内容模型或 dual-write；
- 不存在 fixed permission strings、`reject_s3_versioning`、旧 `/s3` endpoint、`FakeLs`；
- 不存在自定义 KeepLatest/ExpireAfter；
- S3 handler 不直接访问 PostgreSQL；
- S3/DAV/JSON/Processor 写入都经过 ObjectService；
- Preview、Variant、Short Link 和 Batch 结果均绑定 Object Version；
- 未复制 Silo/MinIO 代码。

### 数据与事务

- `objects` 与 immutable `object_versions` 分离；
- `external_version_id` 在 object_id 范围唯一，不同 Key 可同时为 null；
- 不存 `is_latest`，`became_noncurrent_at` 正确维护；
- Upload Intent/Multipart 暂存不进入版本历史；
- 同 Key Enabled PUT 可保留多个版本；Unversioned/Suspended 只有一个 null version；
- Multipart Complete 原子提交 version/head/quota/outbox/终态；
- 删除事务内重检 Object Lock 并持久化 GC Task；
- GC、Outbox、Quota 和重放测试无漂移。

### 协议与功能

- Versioning 状态机和 Object Lock 不可逆约束符合本文；
- Get/Head/Delete/ListObjectVersions 的 delete marker/null version 行为通过 golden tests；
- Lifecycle 第一阶段只接受已实现子集；
- AWS CLI、boto3、AWS SDK JS v3 通过第一阶段工作流；
- 扩展门禁的 Go SDK、rclone、mc 通过后再宣称广泛 S3 客户端兼容；
- 未支持 operation 返回标准错误，不假成功。

### 最终仓库与质量

- migrations 最终仅保留压平后的新基线，不保留兼容迁移；
- fresh install、repository contract、并发与 failure injection 全通过；
- 文档、OpenAPI、Web UI 和实际 capabilities 一致；
- 以下命令通过：

```text
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
pnpm typecheck
pnpm lint
pnpm build
```

## 23. 默认运行配置

1. S3 独立 listener 默认绑定 `0.0.0.0:9000`，当前通过 `MEDIAHUB_S3_BIND_ADDR` 覆盖；端口不是协议常量。
2. 公开 Host、scheme 和 base endpoint 不写死在代码中。签名校验使用请求的原始 Host；位于反向代理后时，只信任显式配置的代理头和 public endpoint 配置。
3. GC 默认宽限期为 24 小时，通过 `PRISMARK_GC_GRACE_HOURS` 覆盖；测试使用 Fake Clock，不等待真实时间。

这些是实现默认值，不是旧版本兼容约束。其他核心决定已经在本文固定，编码过程中不再为不存在的线上旧版本引入兼容层。
