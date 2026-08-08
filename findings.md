# 研究发现：mediaHub 与 pgsty/silo

## 当前项目：MediaHub

### 产品定位与入口

- `readme.md:3` 将项目定位为“面向 AI 时代媒体产物的自托管对象存储与处理服务”。
- `readme.md:162-225` 列出 JSON 控制面 API、原生路径对象 API、WebDAV、S3 兼容后端和受限 S3 网关。
- `readme.md:241-267` 列出 Application/AccessKey/Bucket 管理、图片/视频/静态文件上传读取删除、Metadata、配额、审计、Webhook、后台管理，以及 Local/S3 后端。

### 技术与实现证据

- `Cargo.toml` 是 Rust 2024 workspace，拆为 core、app、local adapter、image adapter、postgres adapter、S3 adapter、OpenAPI 和 server 八个 crate。
- Rust 依赖包含 Axum、SQLx/PostgreSQL、`object_store` AWS 后端、WebDAV、AES-GCM/Argon2/HMAC、图片处理和 OpenAPI 生成。
- `web/package.json` 显示 React 19 + Vite + TanStack Query + OpenAPI client + Vitest；viewer plugin 包含 PDF、归档、表格和 SQLite 相关能力。
- `readme.md:199-224` 说明 PostgreSQL 保存 Metadata、权限、配额、Variant 和任务状态，Local/S3 保存二进制；形成控制面/数据面分离。
- `readme.md:166-168,902-954` 与代码显示邮箱认证、Session/CSRF、Application AccessKey、HMAC-SHA256、Nonce/Idempotency-Key 和 scoped permissions。
- `readme.md:137,1007-1019` 说明 AccessKey/Webhook Secret 使用独立的 AES-256-GCM 版本化 keyring；Webhook worker 支持投递历史、重试、dead-letter 和 replay。

### 当前真实能力边界（代码优先）

- `crates/mediahub-server/src/handlers_health.rs:139-148` 当前 capabilities：Docker profile、Local/S3 storage、S3 gateway、image processing 开启；video processing、resumable upload、archive restore 关闭。
- `crates/mediahub-server/src/s3_http_support.rs:217-227` 的 `reject_s3_versioning` 对 `versionId` 返回 `NotImplemented`，集成测试也断言该行为。
- `readme.md:217-225` 明确将 `/s3` 定义为受限网关，不是完整 AWS S3 Bucket 管理服务；当前重点是 HeadBucket、Put/Get/Head/Delete、受限 List 和 Multipart。
- `crates/mediahub-core/src/bucket.rs:12-20` 的生命周期规则主要是 `ExpireAfter` 与 `KeepLatest`，不是带版本、保留和远端 tier 的完整 ILM。

## pgsty/silo 外部资料

### 产品边界

- GitHub README 将 Silo 定义为 PGSTY 维护的 MinIO fork，核心目标是持续维护 S3-compatible object storage。
- README quick start 是单进程/容器启动 Silo Server，9000 提供 S3 API、9001 提供 Console；镜像内置 `mcli`。
- README 列出多架构容器、Linux/macOS/Windows amd64/arm64 二进制、RPM/DEB/APK、Kubernetes Helm；发布物包含 checksums、SPDX SBOM、Sigstore 签名 manifest 和 GitHub build attestations。
- 兼容性承诺覆盖 S3 API、`MINIO_*` 配置、`minio_*` 指标、`x-minio-*` headers、`/minio/*` 路由、MinIO import paths 和 on-disk format。

### 能力面

- 数据面/可靠性：distributed deployment、availability/resiliency、erasure coding、object healing、object scanner、阈值/限制。
- 对象治理：bucket versioning、object locking/WORM、生命周期过期、远端 tier/transition、压缩。
- 身份与安全：内置用户/组、OIDC、LDAP/AD、外部 identity/access-management plugin；SSE-KMS、SSE-S3、SSE-C、KES 和 TLS。
- 复制与事件：单向/双向/多站点 bucket replication、resync、batch replication；bucket notifications 可接 AMQP、MQTT、NATS、NSQ、Elasticsearch、Kafka、MySQL、PostgreSQL、Redis 和 Webhook。
- 运维/生态：Silo Console、Prometheus/InfluxDB metrics、外部日志/audit、Grafana、Kubernetes Operator/Tenant、裸机/容器/Windows/macOS 安装、硬件/节点/站点故障恢复、多个语言 SDK、STS、Object Lambda、FTP/SFTP。
- `mcli` admin surface 覆盖匿名策略、复制、ILM、legal hold、retention、share URL、批处理、加密、标签、版本、heal/scanner/rebalance/KMS/policy/user/service-account 等。

### 兼容性审计证据

- Silo 官方兼容性审计说明它保持 MinIO S3-facing 与 on-disk compatibility，但 binary、package、service、默认配置目录、container path、Helm resource names、embedded Console、update behavior、部分授权和错误响应已变化。
- 审计记录的 2026-08-06 prepared snapshot 相对 baseline 有 523 files、+36,715/-21,450 行净源代码差异；仓库主页当前 HEAD 为命名切换后的 `100e2e57`。
- 审计记录 137 compatible imports、436 environment names、19 metric namespaces、84 headers、330 routes、58 policy identifiers、9,014 exported symbols，以及 `.minio.sys`、IAM/KMS/replication/healing state 和现有磁盘元数据兼容。
- Silo Console v2.1.1、维护版 MCLI 和 Silo Pkg 已纳入；SUBNET/callhome 被强制关闭，OCI 镜像内置 `/usr/bin/mcli` 并保留 `mc` 客户端兼容别名。

## 差距矩阵摘要

| 维度 | MediaHub 当前 | Silo | 判断 |
|---|---|---|---|
| S3 协议 | 受限网关，支持常用对象操作和 Multipart | MinIO 兼容的完整 S3/admin 生态 | MediaHub 硬差距 |
| 分布式可靠性 | Docker 单 profile + Local/S3 adapter；无自身纠删码/集群修复 | 分布式、纠删码、healing、scanner、rebalance | MediaHub 硬差距 |
| 版本与保留 | `versionId` 明确未实现；无 WORM/legal hold | versioning、object lock、retention/legal hold | MediaHub 硬差距 |
| 复制与分层 | 有异步删除/生命周期，但无 Silo 级 bucket/site replication 与 tiering | 单/双向/多站点复制、resync、远端 tier | MediaHub 硬差距 |
| IAM/企业认证 | 邮箱用户、Application、AccessKey、scope、HMAC | 用户/组/policy、OIDC、LDAP/AD、STS | 两者定位不同，MediaHub 缺企业 IAM |
| 对象加密 | 主要保护 AccessKey/Webhook secret；未见完整 SSE/KMS 对象层 | SSE-KMS、SSE-S3、SSE-C、KES | MediaHub 缺口 |
| 事件 | 面向应用的 Webhook + outbox/retry/replay | Bucket event + 多种消息/数据库/搜索系统 | MediaHub 集成广度差距，但业务闭环更强 |
| 生命周期 | `ExpireAfter`、`KeepLatest`、异步收敛 | ILM、过期、版本、远端 transition/tiering | MediaHub 功能窄但语义更贴媒体业务 |
| 媒体处理/查看 | 图片 Variant/裁剪/格式、Metadata、归档/表格/SQLite/PDF viewer 方向 | 通用对象存储；Object Lambda 不是同类媒体产品 | MediaHub 优势 |
| 应用控制面 | 多 Application、配额、审计、短链、公开/私有签名 URL | 通用 IAM/Console，非媒体 SaaS 控制面 | MediaHub 优势 |
| 交付运维 | Docker Compose/单 Origin 方向；当前 capabilities 为 Docker | 容器、二进制、包、Helm、Operator、丰富运维工具 | Silo 优势 |
| 许可证 | MIT | AGPL-3.0-or-later | MediaHub 对闭源商业集成更友好 |

## 战略判断

- 两者不是同一层产品：Silo 是存储基础设施/数据面，MediaHub 是媒体应用控制面/体验层。
- 不建议把“做成第二个 Silo”作为 MediaHub 的默认路线；分布式存储、全量 S3 兼容、纠删码、修复、复制和运维生态是多年累积的基础设施工程。
- 更有价值的组合是：MediaHub 继续负责用户/Application/权限/配额/Metadata/Variant/分享/Webhook/UI，Silo 作为 S3 数据面或外部存储后端，利用现有 `MEDIAHUB_STORAGE_BACKEND=s3` 连接能力。

## 当前工作区状态

- Git 分支：`master`，跟踪 `origin/master`。
- 用户未提交变更：`sub2api/` 未跟踪；本研究不修改它。
- 本研究生成/更新：`task_plan.md`、`findings.md`、`progress.md`。

## PrismArk S3 实施审计（2026-08-08）

- 当前 `media` 单行模型和 `(application_id, bucket_id, object_key)` 唯一约束无法正确表达覆盖写、并发版本、Delete Marker 和 null version。
- 最小正确模型必须拆成逻辑 `objects` 与不可变 `object_versions`，由 `objects.current_version_id` 指向当前数据版本或 Delete Marker。
- Versioning 状态应为 `Unversioned / Enabled / Suspended`；一旦启用不能回到 Unversioned，Object Lock Bucket 不能暂停版本控制。
- `external_version_id` 的唯一性必须限定在 `object_id` 内，因为不同 Key 都可能拥有 `versionId=null`。
- Staged 普通上传和 Multipart 不得提前进入对象版本历史；最终提交必须原子更新版本、Head、Quota、Outbox 与上传终态。
- Object Lock 的 Retention/Legal Hold 必须在逻辑删除事务内重新检查，物理 blob 删除改由持久化 GC Task 幂等执行。
- Lifecycle 首批仅支持 Expiration、NoncurrentVersionExpiration、ExpiredObjectDeleteMarker、AbortIncompleteMultipartUpload 和 Prefix Filter；不支持的动作必须明确拒绝。
- 推荐顺序：Schema/Core → ObjectService 纵向闭环 → Multipart → Object Lock → Lifecycle → JSON/DAV/Preview/Variant 切换 → 删除旧 Media 并压平迁移。
- 既有方案需修正：开发期间不能先删除全部 migrations/Media，也不能在 Versioning 之前重接 Multipart；最终发布仍不保留双写或旧兼容层。

## PrismArk S3 当前落地结论（2026-08-08）

- S3 已切换为独立 9000 listener，生产 Router 不再保留 `/s3` 入口。
- 普通 Put 与 Multipart 共享 UploadIntent → promotion → ObjectVersion 原子提交链路；新 S3 纵向路径不写 Media。
- Unversioned/Enabled/Suspended、null version、delete marker、精确版本读删和当前 head 重算已经落地。
- ListObjectsV2 只从当前 committed ObjectVersion 读取，prefix/delimiter/cursor 使用同一排序窗口。
- DeleteObject/DeleteObjects 在同一事务中处理版本语义、Object Lock 检查与持久化 GC；Governance bypass 必须是被 SigV4 签名的严格布尔 header。
- Multipart 已使用 Part MD5、标准 multipart ETag、恢复/重放与持久化 GC；Complete 不再创建 Media。
- 当前仍不能宣称完整 S3：缺 CopyObject/UploadPartCopy、ListObjectVersions、Policy、Tagging、Object Lock 修改 API、完整 Lifecycle 执行器、Notification/CORS/SSE 和广泛客户端互操作验证。
- WebDAV 作为产品兼容层已保留，但内部仍待迁移到统一 ObjectService；Preview/Variant 也仍需绑定不可变 object_version_id。

## Docker 真实后端验证结论（2026-08-08）

- 静态 SQL 测试无法替代 fresh database：真实 PostgreSQL 发现约束同名和 `chr(0)` 两个迁移缺陷，均已修复。
- 修复后 PostgreSQL 17 fresh migration、Repository Contract 和 Server 133 个测试全部通过。
- Silo 真实 bucket 证明 generic S3 不应假设普通 CopyObject 支持 destination create-only；PrismArk 现在使用条件 Multipart Copy 建立不可覆盖语义。
- Multipart Copy 不保留 attributes，必须在取得随机不可变 final key 的所有权后执行同源 server-side metadata repair，并在重试时先比较字节，避免把不同对象误判为幂等提交。
- `pgsty/silo:latest` 上的完整 ObjectStore 合同和 Presigned PUT 已通过，验证了 Local/S3 共享端口在真实兼容后端上的关键行为。
# S3 Listing vertical slice findings (2026-08-08)

- Baseline is clean `bdd6323` on `master`.
- Existing ListObjectsV2 is split between `s3_http_core.rs`, `s3_list.rs`, `s3_xml.rs`, and `S3ObjectRepository`.
- `S3ObjectRepository` already exposes a partial object-version listing path used internally, but no S3 ListObjectVersions HTTP contract exists yet.
- Multipart persistence is in `s3_multipart_uploads`; listing must read upload rows and never enumerate object storage.
- `S3ObjectRepository::list_s3_object_versions` currently lists versions for one internal `ObjectId`; the S3 API needs a new bucket-scoped page DTO/query instead of looping over objects.
- Existing `S3ObjectListQuery` validates the 1000-item cap and PostgreSQL ListObjectsV2 already follows the desired limit+1 pagination pattern.
- Existing ListObjectsV2 query parsing/rendering lives in `s3_list.rs`, while generic S3 XML builders live in `s3_xml.rs`; the new version/multipart list XML can share the latter's XML escaping helpers.
- Existing HTTP bucket handler is centralized in `s3_http_core.rs`; classifier query-key allowlists in that file must distinguish `versions` and `uploads` before auth/dispatch.
- ListObjectVersions must page a combined byte-ordered stream: object key ascending, generation descending within a key, with delete markers included and superseded null rows excluded. `objects.current_version_id` determines `IsLatest`.
- ListMultipartUploads must include active `pending`/`completing` rows from `s3_multipart_uploads`, ordered by key then upload ID, and must not call `ObjectStore`.
- A dedicated `S3ListingRepository` in app `s3_repository.rs` avoids expanding the existing per-object audit method and avoids forcing in-memory object-store implementations to emulate PostgreSQL bucket listing.
- The new app types will need a minimal root re-export in `mediahub-app/src/lib.rs`; this is the only anticipated source write outside the requested listing file set.
- `PostgresRepository` is stored concretely in `AppState`, so a new exported trait can be called directly without changing router/state plumbing.
- `object_versions.generation` is the stable per-key chronology; timestamps are presentation fields and must not be used as the pagination tiebreaker.
- Multipart upload IDs are persisted opaque strings, so marker comparison must use PostgreSQL `COLLATE "C"` rather than locale ordering.
- PostgreSQL listing DTO/query code now compiles. Version-marker existence is checked with a parameterized lookup so an unknown paired marker can become S3 `InvalidArgument` instead of silently skipping data.
- Both SQL builders produce delimiter prefixes and concrete rows in one CTE stream, apply markers before `LIMIT`, and return only metadata columns/object-version rows.
- `ListMultipartUploads` filters `state IN ('pending', 'completing')` and `expires_at > as_of`; completed, aborted, and logically expired uploads are invisible.
- HTTP compilation passes after adding pre-auth classification, strict operation-specific parameter rejection, marker parsing, handlers, and XML conversion.
- Standard Multipart behavior is preserved for an `upload-id-marker` without `key-marker`: it is echoed but ignored by the repository cursor.
- The version handler maps an unknown paired marker to `InvalidArgument`; all other repository failures remain service errors.
- Real PostgreSQL 17 execution validates version ordering/resume, null/delete-marker `IsLatest`, delimiter prefixes, invalid marker handling, multipart key/upload ordering, expiry/state filtering, and marker resume.
- The disposable PostgreSQL container used an ephemeral loopback port and was removed after the contract passed; no volume was created.

## 第二阶段实现结论（2026-08-08）

- Silo 源码最终成功克隆到 `.research/silo`，HEAD 为 `100e2e5`；前期“无法克隆”的记录只是当时的网络失败，不再代表当前状态。
- Operation classifier 必须把 `x-amz-copy-source`、`?versions`、`?uploads`、`?object-lock` 以及对象级 lock subresource 放在普通 Put/Get/Multipart 分支之前，否则会出现危险的误路由或假成功。
- CopyObject 与 UploadPartCopy 复用 UploadIntent → ObjectVersion 提交链路；复制结果不能绕过 checksum、版本、quota、GC 和审计边界。
- ListObjectVersions 的 `IsLatest` 只能由 `objects.current_version_id` 推导；版本时间戳不能作为分页游标，稳定顺序使用 key byte order + generation。
- ListMultipartUploads 只列 pending/completing 且未过期的持久化 upload；对象存储扫描既不正确也无法稳定分页。
- WebDAV 作为兼容层可以复用 ObjectVersion 服务，但 MOVE 在没有条件化源删除事务前不应通过“COPY 后普通 DELETE”冒充原子移动。
- 预览接口按 application_id + version_id 查询 committed data version，delete marker 与跨租户统一 404；这比从 legacy Media 反查更符合不可变预览语义。
- 文件浏览器若直接加载原图会把产品差异化能力变成带宽风险；使用规范化 Variant 缩略图、IntersectionObserver 和并发闸门更符合 PrismArk 的定位。
- 当前已通过 Silo 真实 ObjectStore 合同，但本机仍未安装 AWS CLI、mcli/mc 或 rclone，因此不能把 native-client 脚本存在等同于这些客户端已实测通过。
