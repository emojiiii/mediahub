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
- 该阶段仍不能宣称完整 S3；此后 CopyObject/UploadPartCopy、ListObjectVersions、Object Tagging、Object Lock 核心 API 和 Lifecycle 核心执行器已经补齐，当前主要差距转为 Policy、Notification、Bucket Tagging、CORS/SSE、virtual-host style 和广泛客户端互操作验证。
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

## S3 Object Tagging 审计发现（2026-08-08）

- 基线工作区干净，HEAD 为 `72eb4d4`。
- Silo 路由把对象级 `GET/PUT/DELETE ?tagging` 放在普通对象 GET/PUT、CopyObject 与 Multipart 分支之前；PrismArk 必须保持同样的消歧顺序。
- PrismArk 当前 CopyObject 对 `x-amz-tagging` 和 `x-amz-tagging-directive` 一律显式拒绝，不存在静默忽略，但尚未实现 COPY/REPLACE。
- 标签必须成为 ObjectVersion 的独立持久化事实，不能混入 user metadata，也不能附着 delete marker。
- 采用独立 `object_version_tags` 行表最符合边界：`object_version_id + position` 保持响应顺序，`object_version_id + tag_key` 保证唯一键；Application/bucket/key/version 隔离由事务内 join/lock 解析保证。
- PutObject、CopyObject 和 Multipart Complete 的初始标签必须进入 `S3ObjectVersionCommit`，与 ObjectVersion 插入、head 切换、quota/outbox/GC 在同一事务内完成，不能提交后补写。
- 独立 Put/DeleteObjectTagging 应由 Repository 在事务内锁 bucket、logical object 和 exact version，再原子替换标签；Memory 实现需要同等解析和 delete-marker 拒绝语义。
- 标签限制按 S3/AWS 规则实现：最多 10 个、Key 1–128 Unicode 字符、Value 0–256 Unicode 字符、Key 唯一，字符集为 Unicode 字母/数字/空白和 `_ . : / = + - @`。
- 字符校验必须显式排除 Unicode control；`\n`、`\t` 即使满足空白判断也不是合法标签字符，已加入负测。
- `x-amz-tagging` 必须在 HTTP header 边界完成严格 form URL 解码：`+` 表示空格，`%2B` 表示加号，非法百分号、非 UTF-8 与未编码额外等号均拒绝。
- 标签已作为 UploadIntent/MultipartUpload 的独立冻结事实进入 ObjectVersion 原子提交；CopyObject 默认 COPY，REPLACE 使用目标标签，UploadPartCopy 对标签头显式拒绝。
- PostgreSQL 使用独立 `object_version_tags` 表和 delete-marker 复合外键；标签替换在精确版本锁下原子完成，不写入 `user_metadata`。
- 对象级 `?tagging` classifier 位于普通 GET/PUT/Copy/Multipart 之前，GET/HEAD 仅在标签非空时返回 `x-amz-tagging-count`。
- AWS CLI 能严格覆盖 Tagging 正向语义，但 invalid/duplicate/超过 10 个标签与坏 percent-encoding 可能在客户端模型层先失败；没有可审计 raw SigV4 发包器时必须记为 SKIP，不能冒充服务端 PASS。
- 连续预览应以当前可见文件集合为导航边界，并在 item id/revision 变化时重建预览状态；否则旧签名 URL、Variant 或缩放状态会跨对象泄漏。

## 标准 S3 Lifecycle 执行器初始审计（2026-08-08）

- 当前工作树干净且 HEAD 为用户指定的 `2d512fd`；该提交已经包含上一轮 Tagging、预览和兼容矩阵结果。
- Core 已有严格的 `S3LifecycleConfiguration` 模型与 PUT 阶段解析拒绝；执行器应消费规范化配置，不重新解释 XML。
- `ObjectVersion` 已持久化 `became_noncurrent_at`，数据库已有 bucket/time/id 索引，可作为 NoncurrentVersionExpiration 的扫描事实。
- `StorageGcReason` 已包含 `LifecycleExpiration`，但现有对象删除路径当前固定使用 `ExplicitDelete`；Lifecycle 不能直接冒充普通 DeleteObject 调用。
- PostgreSQL 已有 `delete_s3_object_in_transaction` 与 Object Lock 检查，后续设计必须提取/参数化事务内删除语义，而不是在 worker 中直接删行或扫描对象存储。
- `run_lifecycle_worker` 目前只处理旧 Media 生命周期、上传会话和全局 `expires_at` Multipart；标准 S3 Lifecycle 尚未接入，物理对象删除应继续交给独立 `run_s3_storage_cleanup_worker` 消费持久 GC task。
- Bucket S3 configuration 已有单调 `revision`，Lifecycle PUT、Versioning 与 Object Lock 配置变更都会推进 revision，正好可作为“扫描后执行前”事务 fencing token。
- 现有 Multipart `abort_upload_and_enqueue_cleanup` 会在事务内把 upload 设为 aborted，并为 part/attached intent 写持久 GC；Lifecycle 应复用这一终止语义并增加 bucket/prefix/config-revision/initiated-at 条件重检。
- 现有 `expire_multipart_uploads` 只按 upload 自身 `expires_at` 扫描，不能表达 bucket Lifecycle Rule 的 prefix 与 DaysAfterInitiation；标准执行器需要独立候选端口，不能拿全局 TTL 冒充 Lifecycle。
- Core Lifecycle 模型当前仅包含 Empty/Prefix filter、Expiration、NoncurrentVersionExpiration 与 AbortIncompleteMultipartUpload；transition/tag/size filter 不进入模型，符合 PUT 阶段显式拒绝要求。
- 普通 `DeleteS3ObjectCommand` 同时承载外部 DeleteObject 的 marker ID、bypass flag 和 `ExplicitDelete` GC reason，直接复用该命令会破坏 Lifecycle 的 expected-current/config-revision fence 与 reason；需要 Lifecycle 专用执行命令和 outcome。
- Lifecycle current expiration 在 Enabled bucket 必须锁定扫描时的 `current_version_id`，重检后才创建 marker；Suspended/Unversioned 也要带 expected-current，随后复用对应 null-version 删除语义。
- Noncurrent/Expired-marker 应以内部 `ObjectVersionId` 精确定位；前者删除后不能把旧版本提升为 current，后者仅允许 current marker 且确认除 marker 外无 active version。
- Object Lock 判定函数已经满足“legal hold 优先、未到期 Governance/Compliance 均阻止；bypass=false 时 Governance 不可删除”。Lifecycle 专用路径应始终以 bypass=false 调用同一判定逻辑。
- 持久 GC 对同一 storage key 已具备幂等约束；需将 helper 的 reason 参数化并把 Lifecycle task identity 纳入精确重复判断。
- App 层已适合承载生命周期编排：`Clock` 可注入 Fake Clock，Repository 只负责元数据候选与事务执行；UTC Days 截止采用“今天 00:00 UTC 减去 days”，并用候选时间严格 `< cutoff`，从而实现 S3 的次日午夜边界。
- 最终 batch 以“一次 bucket/action 候选查询”为预算单位，空结果同样消费预算且查询固定 `LIMIT 1`；cursor 的 action round 在 current、ordinary/EODM marker、noncurrent、multipart 与后续 bucket 间轮转，事务 fencing/幂等检查仍是最终保障。
- Memory repository 原先把 Lifecycle replacement 留为 `unreachable!`，本切片必须补成真实 revision 更新，否则无法验证配置竞争场景。
- PostgreSQL 执行器必须与 Multipart 用户请求保持同一锁序，并只锁定 action 所需的 current/exact/null/marker 版本；不能在单个 key 上无界锁定全部活跃版本。配置 revision/rule、expected current 或 exact version、noncurrent timestamp、Object Lock 都要在变更前重检。
- current expiration 在 Enabled 只创建 marker、不回收旧 data；Suspended 只回收 active null 并创建 null marker；Unversioned 回收 null data 并清空 head，均沿用现有删除语义。
- expired marker 路径要求 current pointer 命中且 active version 数量恰为 1；Noncurrent 路径不会提升旧版本为 head。
- Lifecycle Multipart scanner 排除仍持有 completion lease 的上传；执行时再次锁 upload 并检查 lease，过期 completing 可复用现有 attached-intent/parts 持久清理原语。
- 现有 PG contract 统一使用专用 `MEDIAHUB_TEST_POSTGRES_URL`、迁移后 `TRUNCATE users CASCADE`、真实 S3ObjectService 写版本；Lifecycle contract 可复用同一模式验证元数据、GC reason 和事务竞争，而无需访问真实对象存储。
- 最终实现没有新增对象存储枚举/扫描：Lifecycle worker 只分页读取 bucket/config 与 PostgreSQL metadata 候选，物理 blob 仍由既有持久 GC lease/retry worker 清理。
- 实际竞争验证覆盖 scan→execute 窗口：配置 revision 变化返回 `ConfigurationChanged`、current head 被新 PUT 替换返回 `TargetChanged`；Enabled current Expiration 只创建 marker，不受现有 data version Retention 阻止，而受保护 noncurrent exact version 的永久回收返回 `Locked`。
- PG17 合同证明对象过期 GC reason 为 `lifecycle_expiration`，Multipart part 仍使用其专属 `multipart_temporary`，没有冒充 `explicit_delete`。
- Enabled current expiration 只新增 delete marker；随后 NoncurrentVersionExpiration 精确回收 data version，最后只有 current marker 时 ExpiredObjectDeleteMarker 才移除 marker。Memory 另行验证 Unversioned 清空 head、Suspended 创建 null marker。
- Memory adapter 现也完整模拟过期 `Completing` Multipart 的 attached UploadIntent 清理：intent 转为 Aborted，temporary/final 与 part keys 都写入 `MultipartTemporary` GC；与 PostgreSQL 生产实现一致，不再长期返回 Busy。

## PostgreSQL Lifecycle 事务安全复审（2026-08-08）

- 现实现对所有 Lifecycle action 都先 `buckets FOR SHARE`；Multipart Complete 则先锁 upload，再进入 bucket/object 提交边界，因此 Lifecycle Abort 的 `bucket -> upload` 与用户 Complete 的 `upload -> bucket` 构成 ABBA。修复方向是仅 Multipart Lifecycle 改为 `upload -> bucket`，并在 bucket 锁下重做 revision/rule fence。
- `lock_active_versions` 对同一 object 的全部 active committed versions 执行无 LIMIT `FOR UPDATE + fetch_all`。Object row 已先锁，按 action 精确锁 current/exact/null/marker 足以保持 head fence；EODM 只需再做 active sibling 存在性确认。
- PostgreSQL S3 路径目前没有任何 `applications.used_bytes/reserved_bytes` 更新：普通 Put/Multipart 建立和提交都不记账，普通 S3 delete 也不减账。现阶段若 Lifecycle 单方面扣减会破坏账本；安全目标应是 contract 证明 Lifecycle 不使 quota 漂移，并把“先统一接入全部 S3 写入/删除 quota”记录为独立阻塞。
- 现有 Lifecycle 候选 SQL 使用 `LEFT(key, length(prefix))`，current/marker join 缺 joined-version bucket predicate；0013 索引列序和查询 ORDER BY 不完全一致。可用转义后的 `LIKE prefix% ESCAPE '\\'` 配合 `text_pattern_ops`，并让 partial index 与固定状态谓词和排序列一致。
- App 已新增普通 Expiration 对 sole delete marker 的调度入口。PG 必须显式 override：Days 只扫描 `marker.created_at < UTC-days-cutoff`，Date 仅在配置日期到达后扫描，显式 EODM 无时间门槛；执行期必须在锁 marker 后以 marker 时间调用 `_at` helper。
- 完整 Multipart 顺序现在统一为 `upload -> attached intent -> bucket`。虽然同一 upload 的排他行锁已能序列化 Complete/Lifecycle，显式 prelock intent 仍可消除未来 helper/旁路造成的 intent/bucket 逆序，并让锁顺序成为代码结构不变量。
# 2026-08-08 S3 quota / policy implementation findings

- PrismArk 的 S3 ObjectVersion 写入统一经过 `S3ObjectService::begin_put`：PutObject、CopyObject、CompleteMultipartUpload 和 WebDAV ObjectVersion 写入均可复用同一 intent reservation，因此配额不需要在 HTTP handler 各自实现。
- PostgreSQL 配额锁序应保持 intent/upload（若存在）→ bucket → object/version（若存在）→ application；创建新 intent/multipart 时从 bucket → application 开始。当前切片没有发现 application → bucket 的反向获取。
- Silo 的 hard bucket quota 在数据面超额时同样返回 S3 `EntityTooLarge`；PrismArk 将 `RepositoryError::QuotaExceeded` 映射为 `EntityTooLarge` 与该参考行为一致，不需要另造非标准错误码。
- 生命周期和普通永久删除只有在 data ObjectVersion 进入逻辑永久删除时才释放 used bytes；delete marker 为 0 字节，Enabled 下仅创建 marker 不释放历史 data，null replacement 则在同一事务按 `new - old` 转移。
- Bucket Policy 持久化切片采用数据库 identity 生成稳定 12 位账户 ID，并用不可变 trigger 防止重写；bucket 名从 tenant 内唯一提升为全局唯一，符合匿名 path-style 先按 bucket 定位 owner/policy 的前置要求。
- 现有 PostgreSQL unique violation 已统一映射为 `RepositoryError::Conflict`，因此全局 `buckets_name_key` 不需要在 create bucket 路径增加基于旧 constraint 名称的兼容分支。
- Policy persistence 的 UPDATE 直接锁 bucket 行并从 applications 只读 owner account，不取得 application 行锁；这保持 bucket-first，不会和 S3 quota 的 application-last 形成 ABBA。
- 当前 Access Key 的 9 项 permissions 同时服务 PrismArk JSON API/WebDAV/S3；直接删除字段会破坏非 S3 产品能力。S3 对齐应新增标准 identity-policy 语义并让 S3 授权器消费它，现有 permissions 继续作为 JSON/WebDAV 的独立权限模型，而不是作为 Bucket Policy 的 fallback。
- 当前 `load_s3_authentication` 把“解析 SigV4、查 key、验签、构造 ApplicationAuth”绑定在一起，并且所有 S3 路径随后直接调用 `ApplicationAuth::authorize`。支持匿名 Bucket Policy 需要拆成可选 principal 认证与统一 S3 policy authorization；存在 Authorization 但签名无效时必须失败，绝不能降级匿名。
- AWS 官方 API：PutBucketPolicy 的规范 Response Syntax 是 200 empty（旧示例仍展示 204），DeleteBucketPolicy 明确为 204，GetBucketPolicy 为 200 JSON policy；本实现采用规范表定义的 200/204。
- AWS 官方 REST `GetBucketPolicyStatus` 返回 XML `PolicyStatus/IsPublic`；Boto3 文档展示的 JSON 是 SDK 反序列化结构，HTTP handler 不能误做 JSON。
- Bucket Policy 管理要求调用身份属于 bucket owner account；权限不足为 403，权限正确但跨 owner 为 405。`x-amz-expected-bucket-owner` 不匹配始终为 403。
- 参考：AWS PutBucketPolicy https://docs.aws.amazon.com/AmazonS3/latest/API/API_PutBucketPolicy.html ，GetBucketPolicy https://docs.aws.amazon.com/AmazonS3/latest/API/API_GetBucketPolicy.html ，DeleteBucketPolicy https://docs.aws.amazon.com/AmazonS3/latest/API/API_DeleteBucketPolicy.html ，GetBucketPolicyStatus https://docs.aws.amazon.com/AmazonS3/latest/API/API_GetBucketPolicyStatus.html 。
- README 与 `docs/s3-compatibility.md` 仍把完整 S3 quota 和 Bucket Policy 标为未实现；Policy HTTP/授权接线通过后必须更新能力矩阵与兼容测试边界，不能让产品文档继续低估或误报当前状态。
# S3 Policy 全 handler action 映射审计（2026-08-08）

主线按当前协议分类器与 Core `S3PolicyAction` 审查得到以下接线边界；首批 anonymous 数据面只覆盖前三项，不能据此宣称全部 Policy enforcement 完成。

| 协议操作 | 必须求值的 action | 关键条件 |
|---|---|---|
| GetObject / HeadObject | 无 `versionId` 为 `GetObject`，有 `versionId` 为 `GetObjectVersion` | object ARN、VersionId |
| ListObjectsV2 / v1 | `ListBucket` | prefix、delimiter、max-keys |
| HeadBucket | `ListBucket` | bucket ARN |
| ListBuckets | `ListAllMyBuckets` | account resource `*`，仅 Identity Policy |
| CreateBucket | `CreateBucket` | account resource `*`，仅 Identity Policy；创建后不能拿目标 bucket policy 反向授权 |
| ListObjectVersions | `ListBucketVersions` | prefix、delimiter、max-keys |
| ListMultipartUploads | `ListBucketMultipartUploads` | prefix、delimiter、max-uploads 当前 Core 尚无独立 key，未知 key 不得伪装支持 |
| Get/Put Versioning、Lifecycle、Bucket Object Lock、Policy/PolicyStatus、DeleteBucket | 各自同名 bucket action | owner-only 特例与标准 policy action 都要满足既定协议顺序 |
| PutObject、CreateMultipartUpload、UploadPart、CompleteMultipartUpload | `PutObject` | object ARN；Multipart 生命周期所有阶段绑定同一目标 key |
| CopyObject | source `GetObject`/`GetObjectVersion` + target `PutObject` | 两个 bucket/tenant 分别求值，任一拒绝则整体拒绝 |
| UploadPartCopy | source `GetObject`/`GetObjectVersion` + target `PutObject` | 同 CopyObject，不能只验证 upload 所属 target |
| DeleteObject / DeleteObjects item | 无 version 为 `DeleteObject`，显式 version 为 `DeleteObjectVersion` | batch 每项独立返回错误；治理绕过另需 `BypassGovernanceRetention` |
| Get/Put/Delete Object Tagging | 根据 `versionId` 选择普通或 Version action | VersionId；不能以 `GetObject` 代替 tagging action |
| Get/Put Object ACL | `GetObjectAcl` / `PutObjectAcl` | 当前仅私有 ACL 形状也必须授权 |
| Get/Put Retention、LegalHold | 各自 action | exact/current version；治理绕过为额外 action，不替代 mutation action |
| AbortMultipartUpload / ListParts | `AbortMultipartUpload` / `ListMultipartUploadParts` | object ARN 与 upload 所属 bucket/key 一致 |

认证顺序不变量：完全没有任何认证材料时才可构造 Anonymous；存在 Authorization、任一 `X-Amz-*` credential/signature query 或不完整签名时必须走认证错误，绝不能降级匿名。signed principal 的 account ID 来自调用方 access key 所属 application，不来自目标 bucket。无 Identity Policy 为 implicit deny，不回退旧 permissions。

## AWS 条件权限复核（2026-08-08）

- AWS 官方权限表确认：`PutObject` 始终要求 `s3:PutObject`；请求同时设置 ACL、标签、Retention 或 Legal Hold 时，还必须分别具备 `s3:PutObjectAcl`、`s3:PutObjectTagging`、`s3:PutObjectRetention`、`s3:PutObjectLegalHold`，这些是叠加权限而不是替代权限。
- `CopyObject` 对源端要求 `s3:GetObject`，显式 `versionId` 时改为 `s3:GetObjectVersion`；目标端始终要求 `s3:PutObject`。复制请求写入目标 ACL、标签、Retention 或 Legal Hold 时，同样分别叠加相应的 Put 权限。
- `CreateMultipartUpload` 始终要求 `s3:PutObject`；初始化请求携带 ACL、标签、Retention 或 Legal Hold 时，同样叠加相应权限。`UploadPart` 只要求目标 `s3:PutObject`；`UploadPartCopy` 另对源端要求 GetObject/GetObjectVersion。
- `PutObjectRetention` 在请求声称绕过 Governance 时，必须在 `s3:PutObjectRetention` 之外再允许 `s3:BypassGovernanceRetention`。`DeleteObject`/`DeleteObjects` 删除受 Governance 保护的版本时也需要额外 bypass action；普通删除与显式版本删除分别是 `s3:DeleteObject` 和 `s3:DeleteObjectVersion`。
- `ListMultipartUploads` 与 `ListParts` 分别是 `s3:ListBucketMultipartUploads` 与 `s3:ListMultipartUploadParts`；不能用 `ListBucket` 或 `GetObject` 泛化替代。
- 参考：AWS 官方权限映射 https://docs.aws.amazon.com/AmazonS3/latest/userguide/using-with-s3-policy-actions.html ，PutObject https://docs.aws.amazon.com/AmazonS3/latest/API/API_PutObject.html ，CopyObject https://docs.aws.amazon.com/AmazonS3/latest/API/API_CopyObject.html ，CreateMultipartUpload https://docs.aws.amazon.com/AmazonS3/latest/API/API_CreateMultipartUpload.html 。
