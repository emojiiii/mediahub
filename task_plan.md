# 对比研究计划：mediaHub 与 pgsty/silo

## 目标

对比当前 mediaHub 项目与 GitHub 上 pgsty/silo 的功能、架构和产品能力，识别功能差距、关键功能点差距，并总结 mediaHub 的现有优势。

## 阶段

- [completed] 1. 盘点当前项目的产品边界、功能和技术结构
- [completed] 2. 获取并盘点 pgsty/silo 的源码、文档和发布能力
- [completed] 3. 建立功能矩阵并判断实际差距与优先级
- [completed] 4. 总结当前项目优势、风险和建议路线
- [completed] 5. 复核证据并向用户交付中文结论

## 研究口径

- 以当前工作区代码和配置为准；不假设未实现的功能已经存在。
- 以 silo 当前仓库默认分支、README、官方文档和兼容性审计为准。
- 外部资料写入 findings.md；本文件只保留计划、状态和错误记录。

## 错误记录

| 错误 | 尝试 | 处理 |
|---|---|---|
| 读取包含用户目录路径的组合 PowerShell 命令触发 Windows sandbox helper_unknown_error | 1 | 改用工作区内的独立读取命令，并避免重复原命令 |
| Git shallow clone 在 Schannel 阶段失败 | 1 | 改用 GitHub README、Silo 官方 Docs 和兼容性审计进行核对 |
| apply_patch 对已创建的研究文件持续触发 ACL helper 错误 | 多次 | 仅对研究记录文件使用受控 PowerShell UTF-8 写入；未改动业务源码 |

## 文件变更

- 新增/更新 `task_plan.md`、`findings.md`、`progress.md`，均为本次研究记录。
- 未修改业务源码；未触碰用户未跟踪的 `sub2api/`。

---

# PrismArk 产品化与 S3 对齐实施计划

## 目标

将产品品牌从 Arkivue/MediaHub 统一为 PrismArk（万象仓），生成并接入品牌 Logo，建设可直接用于 SEO 的响应式官网，并在不复制 Silo AGPL 代码的前提下，按照既有 S3 修改方案补齐当前网关的核心后端能力与协议测试。

## 实施阶段

- [completed] 1. 审计现有工作区、S3 网关、迁移方案和测试边界，拆分并行任务
- [completed] 2. 完成 PrismArk 品牌迁移与 Logo 资产生成、检查和接入
- [completed] 3. 实现官网、结构化数据、SEO 元信息、robots 与运行时 canonical
- [completed] 4. 实现 S3 核心纵向闭环：Bucket、Versioning、Lifecycle 配置、对象版本、删除保护与 Multipart
- [completed] 5. 集成并补充单元、集成、协议与浏览器测试
- [completed] 6. 完成构建、格式、SEO、Compose 与差异检查并交付

## 范围约束

- 不复制 Silo/MinIO 的 AGPL 源码；只参考协议处理顺序、模块边界和测试组织。
- 不保留 Arkivue 或旧浏览器存储键兼容层；项目当前无线上用户。
- 不承诺在单轮内重写分布式纠删码、跨站复制、企业 IAM/KMS 等多年基础设施能力；优先完成当前仓库架构内可验证的 S3 协议闭环。
- 保护工作区已有未提交改动；并行代理不得回滚或覆盖其他代理文件。

## 错误记录（本轮）

| 错误 | 尝试 | 处理 |
|---|---|---|
| Windows sandbox helper_unknown_error 阻止读取技能和仓库文件 | 1 | 使用严格限定目标的只读提升权限命令继续；不扩大写入范围 |
| 首个任务计划补丁 hunk 行数错误 | 1 | 重新生成带正确行数的补丁，不重复使用损坏补丁 |
| PostgreSQL `repository_contract` 仍引用已删除的 Media-based Multipart API | 1 | 不恢复兼容 API；物理删除旧契约并以新的 ObjectVersion/Server 集成覆盖替代，测试目标重新可编译 |
| WebDAV/sqlx 运行时测试最初缺少 `DATABASE_URL` | 1 | 用户启动 Docker 后使用无持久卷的 PostgreSQL 17 临时容器完成真实测试；Server 133/133、Repository Contract 1/1 通过 |
| fresh migration 中自动与显式 state constraint 同名，且 `chr(0)` 检查会阻断 INSERT | 2 | 为值域约束使用独立名称；删除 PostgreSQL `text` 不需要且不可执行的 NUL 检查，重建数据库复验通过 |
| Silo 实测不支持未配置的 `copy_if_not_exists` | 1 | 启用条件 Multipart Copy，并在取得目标所有权后恢复 metadata；Silo 真实 ObjectStore + Presigned PUT 合同通过 |
# PrismArk S3 Listing vertical slice (2026-08-08)

## Goal

Implement ListObjectVersions and ListMultipartUploads from PostgreSQL metadata only, with stable S3 marker, delimiter, encoding and pagination semantics. Do not commit or push.

## Phases

- [completed] Audit existing repository, classifier, list XML and multipart schema.
- [completed] Add app DTOs and parameterized PostgreSQL limit+1 queries.
- [completed] Add HTTP classification, query parsing and XML rendering.
- [completed] Add PostgreSQL static tests and HTTP golden/unit tests.
- [completed] Run fmt, focused tests and strict clippy; document uncovered edges.

## Final validation

- `cargo fmt --all -- --check`: pass.
- `cargo check -p mediahub-server --bin mediahub-server`: pass.
- HTTP listing unit/golden tests: 5/5 pass.
- PostgreSQL lib/static tests: 27/27 pass.
- Real PostgreSQL 17 listing contract: 1/1 pass.
- Strict app/PostgreSQL clippy: pass.
- Server clippy: listing slice passes; unqualified workspace command is blocked only by two concurrent CopyObject `collapsible_if` findings, and passes with that foreign lint explicitly allowed.
- `git diff --check`: pass.
- No commit and no push.

## Constraints

- No Media reads/writes or storage-backend scans.
- Keep business-code changes in S3 listing/repository/XML files.
- No git commit or push.

## Decisions

- Use one bucket-scoped `S3ListingRepository` for both APIs.
- Version order is key (`COLLATE "C"`) ascending, generation descending; `current_version_id` is the only `IsLatest` source.
- Multipart order is key (`COLLATE "C"`) ascending, upload ID (`COLLATE "C"`) ascending; only non-expired pending/completing rows are visible.
- Prefix/common-prefix entries share the same SQL page window and count toward the requested maximum.

## Errors

| Error | Attempt | Resolution |
|---|---:|---|
| `s3_http.rs` import patch context was stale because CopyObject imports are present at the baseline | 1 | Re-read the exact import block and apply a narrower patch preserving CopyObject symbols |
| `cargo fmt --all -- --check` also reported concurrent Copy/WebDAV files outside this slice | 1 | Apply formatting only to listing-owned hunks now; do not rewrite concurrent files, then rerun after their changes settle |
| Disposable PostgreSQL `docker run` timed out while Docker Desktop was pulling/starting the image | 1 | Inspect exact container/image state before choosing a non-repeating recovery action |
| Fixed host port `127.0.0.1:55439` is reserved/forbidden on this Windows host | 1 | Let Docker allocate an ephemeral loopback port, then discover it with `docker port` |
| Workspace-wide strict clippy is blocked by two `collapsible_if` findings in concurrent untracked `s3_http_copy.rs` | 1 | Do not edit the concurrent Copy slice; run strict clippy for app/PG and server with only those two foreign lints explicitly allowed, then retry unqualified strict clippy at final state |

---

# PrismArk 第二阶段并行收口（2026-08-08）

## 当前目标

在本地基线提交 `bdd6323` 之上继续补齐高价值 S3 兼容能力，并同步强化 ObjectVersion 预览和 Win11 文件浏览体验；所有验证完成后只做本地提交，不 push。

## 状态

- [completed] 将第一阶段提交为 `bdd6323 feat: launch PrismArk and complete S3 object core`，未 push。
- [completed] 将 Silo 源码克隆到被 Git 忽略的 `.research/silo`，只参考协议顺序、模块边界和测试思路。
- [completed] 完成 CopyObject、UploadPartCopy、ListObjectVersions、ListMultipartUploads。
- [completed] WebDAV 普通文件路径迁移到 ObjectVersion；COPY 可用，MOVE 安全地明确拒绝。
- [completed] 完成 Bucket Object Lock Configuration 与不可变 ObjectVersion 预览后端。
- [completed] 新增 AWS CLI、mcli/mc、rclone 可重复兼容矩阵，并把已完成能力改成必须 PASS。
- [completed] Win11 瀑布流加入视口懒加载、并发受限的 Variant 图片缩略图。
- [completed] 对象级 Retention / Legal Hold、PutObject lock headers 与默认 Retention。
- [completed] 栅格图片预览缩放、平移、适应窗口与键盘交互。
- [completed] 统一 PostgreSQL、Silo、Rust、OpenAPI、前端、格式与 Clippy 回归。
- [completed] 更新最终差距说明并创建第二个本地提交 `72eb4d4`；保持不 push。

## 范围边界

- 不复制 Silo 的 AGPL 实现。
- 不恢复旧 `/s3` 路由、旧 schema 或旧品牌兼容层。
- 视频 Variant 继续留给独立异步服务与队列。
- 该阶段未完成的 Policy、Lifecycle、Notification/CORS/SSE 必须明确披露；后续 Lifecycle 核心执行器已进入独立纵向切片，历史记录不冒充当时已支持。

---

# S3 Object Tagging 纵向切片（2026-08-08）

## 目标

基于干净提交 `72eb4d4` 实现绑定不可变 ObjectVersion 的标准 S3 Object Tagging，覆盖独立 tagging API、Put/Copy/Multipart 写入边界、响应计数、Memory/PostgreSQL repository，以及真实 PostgreSQL 17 与 SigV4 HTTP 验证；不提交、不推送。

## 阶段

- [completed] 1. 审计 operation classifier、ObjectVersion schema/repository 和 Put/Copy/Multipart 提交点
- [completed] 2. 实现 core/app DTO、校验、Repository 与 Memory/PostgreSQL 持久化
- [completed] 3. 实现对象 tagging API 和 Put/Copy/Multipart 标签语义
- [completed] 4. 补齐 GET/HEAD tagging-count、标准 XML/错误与隔离测试
- [completed] 5. 执行真实 PostgreSQL 17、SigV4、全量回归、strict clippy/fmt/diff 并清理容器

## 约束

- 只参考 `.research/silo` 的 operation 顺序、模块边界和测试组织，不复制 AGPL 代码。
- 不改 `web`、`readme.md`、兼容脚本；不 commit、不 push。
- 标签属于 ObjectVersion，不写入 `user_metadata`，不允许 delete marker 承载标签。

## 错误记录

| 错误 | 尝试 | 处理 |
|---|---:|---|
| 首次聚合检索包含不存在的根级 `migrations/` 路径，`rg` 返回退出码 1 | 1 | 改为先定位仓库实际 migration 目录，再执行定向检索 |
| 首次追加规划记录的补丁上下文使用了终端乱码文本，未匹配文件 | 1 | 以 UTF-8 读取精确尾部后，用稳定中文上下文追加 |
| Windows PowerShell 未展开传给 `rg` 的 `s3_http*.rs` 文件通配符 | 1 | 使用 `rg -g 's3_http*.rs'` 的原生 glob 过滤 |
| 首次真实 PG contract 只设置了 `DATABASE_URL`，测试要求专用 `MEDIAHUB_TEST_POSTGRES_URL` | 1 | 设置仓库约定的专用变量后重新执行，contract 与 SigV4 用例均通过 |

## 最终验证

- Core 54/54、App 45/45、PostgreSQL adapter 28/28、Server 165/165 通过。
- 真实 PostgreSQL 17 Object Tagging contract 1/1 通过。
- SigV4 HTTP 巨型往返用例 1/1 通过，覆盖 Put/Get/Delete Tagging、MD5、计数、Copy COPY/REPLACE 与 Multipart 标签冻结。
- `cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --check`、`git diff --check` 通过。
- 临时 PG17 容器使用 tmpfs，已删除且未创建卷；未 commit、未 push，HEAD 保持 `72eb4d4`。

---

# PrismArk 第三阶段并行收口（2026-08-08）

## 状态

- [completed] 完成版本级 S3 Object Tagging 全纵向切片和独立 PostgreSQL 合同。
- [completed] 完成 Object Lock 与 Object Tagging 的 AWS CLI 严格兼容矩阵；本机缺客户端时准确报告 SKIP。
- [completed] 完成对象预览上一项/下一项、位置计数、方向键与状态隔离。
- [completed] Rust 全工作区、PG17、严格 Clippy/Fmt、前端 182/182、OpenAPI 和生产构建统一回归。
- [completed] 更新产品/修改文档并创建第三个本地检查点；保持不 push。

---

# 标准 S3 Lifecycle 执行器纵向切片（2026-08-08）

## 目标

基于干净提交 `2d512fd`，复用现有 `S3LifecycleConfiguration`、`ObjectVersion`、`delete_s3_object`、GC 和 Object Lock 事务边界，实现有界、幂等、竞争安全且不扫描对象存储的数据生命周期执行器；不 commit、不 push。

## 阶段

- [completed] 1. 审计 worker、Lifecycle 配置 revision、对象删除/Multipart 清理与 Object Lock 事务边界
- [completed] 2. 定义 Lifecycle 扫描候选、执行命令、时钟与 Memory/PostgreSQL repository 端口
- [completed] 3. 实现 current expiration、noncurrent expiration、expired delete marker 与 multipart abort
- [completed] 4. 接入有界 worker，并补齐 fake clock、表驱动、竞争/配置变更/Object Lock 测试
- [completed] 5. 执行真实 PostgreSQL 17 contract、全量相关 tests、strict clippy/fmt/diff 并精确清理容器

## 约束与关键不变量

- 仅参考 `.research/silo` 的行为、模块边界和测试思路，不复制 AGPL 代码。
- 不编辑 `web`、`readme.md`、`docs/s3-compatibility.md`、`scripts`；不 commit、不 push。
- 扫描只产生候选；执行必须在同一事务重检配置 revision、current/exact version 与 Object Lock/Legal Hold。
- Lifecycle 永不 bypass Governance；竞争或配置变化只能安全 skip/retry，不能删除新 head。
- GC reason 必须使用 `LifecycleExpiration`；worker 有界、幂等且不得扫描对象存储。
- 现有普通 DeleteObject 语义保持不变；Lifecycle 使用独立候选/执行命令，但内部复用相同的版本隐藏、head 迁移、Object Lock 与 GC 入队原语。
- 先完成可编译纵向切片再扩充测试：App、PostgreSQL test target 与 Server binary 已分别编译通过。

## 终态并发安全复审

- [completed] 只读复审确认配置 revision、head/exact-version fencing、Object Lock 与 GC 原子性基本正确。
- [completed] 统一 Lifecycle Multipart Abort 与 Complete 的 `upload → intent → bucket` 锁顺序，消除 ABBA 死锁。
- [completed] 将单 key 全版本无界锁改为 action 所需的精确版本锁，并优化候选索引。
- [completed] 让 scan/query 本身消耗 batch 预算并改善 current/noncurrent/multipart/bucket 公平性。
- [completed] 补齐普通 Days/Date Expiration 清理到期 sole delete marker 与 UTC 午夜 marker 时间。
- [pending] 对 S3 全链路 quota 事实做独立闭环；禁止只在 Lifecycle 删除侧单边扣减。
- [completed] 完成独立 PG17、workspace、Clippy、fmt、diff 复验。
- [completed] 创建本地提交，不 push。

## 错误记录

| 错误 | 尝试 | 处理 |
|---|---:|---|
| Windows PowerShell 未展开传给 `rg` 的 `crates/.../src/*.rs` 通配符 | 1 | 改用目录参数配合 `rg -g '*.rs'`，不重复原命令 |
| 并行只读聚合中一个 `rg` 无匹配返回 1，包装器中止其余输出 | 1 | 后续将无匹配返回码 1 视为正常，并拆分为确定文件/行段读取 |
| App test target 首次编译缺少 Memory Lifecycle 的 `StorageGcTaskId` import | 1 | 补充 core 类型 import 后重新编译，不改设计 |
| worker 模块检索命令的 PowerShell 双引号转义不完整 | 1 | 改用单引号固定文本检索，不重复原转义形式 |
| worker 接线补丁命中后方 `match`，Lifecycle block 被插入 stale-upload match 内 | 1 | 读取精确行段后移除误插块，并插入主 lifecycle loop 的 Multipart expiry 之后 |
| Fake Clock 测试模块缺少 `S3LifecycleConfiguration` import | 1 | 只补测试 import 后重跑针对性测试 |
| 补测试 import 的首个 patch 含空 hunk，校验失败 | 1 | 删除空 hunk，使用单一有效上下文重新应用 |
| batch cursor 修正 patch 再次误带空 hunk | 1 | 移除空 hunk，并在后续 patch 中禁止无上下文的文件切换 |
| PG lib 旧静态测试只截取 `delete_lock_reason`，重构后锁逻辑位于 `delete_lock_reason_at` | 1 | 更新静态断言检查共享 helper；运行时 PG17 contract 已通过 |
| Server `lifecycle` 过滤词额外命中无关认证测试，因缺 `DATABASE_URL` 失败 | 1 | 精确过滤 `s3_http::s3_lifecycle`；全量回归显式设置临时 PG17 URL |
| 首次 `cargo fmt --check` 报告新文件标准格式差异 | 1 | 运行 `cargo fmt --all` 做机械格式化后重检 |
| 最终范围守卫发现工作期间出现并发 `docs/s3-compatibility.md` / `scripts/s3-compat/**` 改动 | 1 | 确认本切片未编辑这些文件；不回退并发成果，最终单独披露；Lifecycle 自有文件范围继续审计 |
| 新增 Memory 分支测试使用了旧 helper 未内置 MD5 的新字节串 | 1 | 改用现有 `null-version` 固定向量后重跑；失败发生在 Lifecycle 执行前 |

---

# PostgreSQL S3 Lifecycle 事务安全与资源边界修复（2026-08-08）

## 目标

在不提交、不推送且保留共享工作区改动的前提下，修复 PostgreSQL Lifecycle 执行器的 Multipart 锁序、精确版本锁、配额一致性与 SQL 有界性，并以真实 PostgreSQL contract 验证。

## 阶段

- [completed] 1. 审计 Complete/Abort 锁序、Application quota 与现有删除原语
- [completed] 2. 重构 Lifecycle 事务为按 action 精确锁定并消除 ABBA
- [completed] 3. 实现可证明的 used/reserved quota 对账或记录阻塞证据
- [completed] 4. 对齐候选查询与 0013 索引，补充 PostgreSQL contract
- [completed] 5. 运行 targeted PG17、fmt、clippy 与差异检查

## 写入约束

- 业务源码限定 `crates/mediahub-adapter-postgres/src/s3_lifecycle.rs`、`migrations/0013_s3_lifecycle_executor.sql`、`tests/s3_lifecycle_contract.rs`。
- quota 若必须修改同 crate 其他文件，先向用户说明证据和必要性；本轮不会 commit/push。
- 共享工作区已有未提交改动全部保留，不回退他人文件。

## 已确认决策

- Multipart Lifecycle 严格遵循既有 Complete 的 `upload -> attached intent -> bucket -> cleanup` 锁序；bucket 锁后再次验证配置 revision 与 rule。
- Object Lifecycle 保持 `bucket -> object`，但每个 action 只锁精确 current/exact/null/marker；不再加载全部历史。
- 当前 S3 全写入链路未接入 Application quota，本轮不修改 quota；以 contract 证明 Lifecycle 前后 `used_bytes/reserved_bytes` 不漂移，并记录完整 S3 quota 为下一切片 blocker。

## 错误记录

| 错误 | 尝试 | 处理 |
|---|---:|---|
| 并行检索中某个 `rg` 无匹配返回 1，导致聚合调用整体无输出 | 2 | 第一次后 quota 检索已归一化；第二次遗漏。后续所有混合聚合改用 `Promise.allSettled`，不再依赖每条命令退出码 |
| 首次 PostgreSQL adapter 编译发现新 SQL 中 `COLLATE "C"` 的 Rust 字符串引号未转义，并暴露精确锁返回 owned version 后的 7 处借用遗漏 | 1 | 改为转义 SQL identifier，并在调用既有 helper 时显式借用；不改变事务设计 |
| 同时更新 plan/progress 的补丁因 progress 行内空格上下文不精确而整体未应用 | 1 | 用 `rg` 读取精确文本后拆分补丁，避免重复原上下文 |
| 首次 `docker exec psql -c` EXPLAIN 被 Windows 参数传递剥离 `COLLATE "C"` 的 identifier 引号 | 1 | 索引定义已成功核对；EXPLAIN 改为经 stdin 传 SQL，避免命令行引号重解释 |
| 最终范围检索再次把“预期无匹配”的 `rg` 放入 `Promise.all`，导致聚合无输出 | 3 | 立即改用 `Promise.allSettled`；此后不再并行聚合任何可能返回 1 的裸 `rg` |

## 验证记录

- 一次性 PostgreSQL 17 fresh migration 成功。
- `s3_lifecycle_contract` 1/1 通过，新增覆盖：Complete-like upload-first 锁序无 ABBA、锁住无关 sibling 不阻塞 exact noncurrent 删除、特殊字符 prefix 精确匹配、current/noncurrent/marker/multipart/idempotent 全程 quota sentinel 不漂移。
- final fresh migration 上 `s3_lifecycle_contract` 再次 1/1 通过，并覆盖 intent/bucket 顺序、UTC marker 动作时间与普通 Days/Date sole marker 到期前后语义。
- `EXPLAIN` 在禁用顺序扫描时确认 current、noncurrent、marker、multipart 与 lifecycle bucket 查询均使用 0013 索引；prefix 改为 Unicode 安全的 C-collation 半开区间后，current/marker 计划不再出现 LIMIT 前 Sort。
- 最终验证：App Lifecycle 4/4、PG adapter lib 28/28、PG17 Lifecycle contract 1/1、Server binary check、adapter all-target strict Clippy、workspace fmt check、git diff check 全部通过。
- 临时 PostgreSQL 17 容器使用 tmpfs、无 volume，已按精确名称删除；本轮未 commit、未 push。

---

# S3 Quota 与 Bucket Policy 并行纵向切片（2026-08-08）

## 目标

基于本地提交 `f004443`，把 S3 Put/Copy/Multipart/版本永久删除/Lifecycle 全链路接入 Application quota，并并行建立标准 Bucket Policy 的严格 Core 模型与 evaluator；不保留旧兼容路径，不 push。

## 阶段

- [completed] 1. 完整审计 S3 intent reserve、commit transfer、null replacement、version delete、Multipart 与 Lifecycle 账本边界
- [completed] 2. 实现 PostgreSQL/Memory quota 原子增减、幂等、竞争与 quota-exceeded 合同
- [completed] 3. 实现标准 Bucket Policy Core parser/validator/evaluator、稳定序列化、PolicyStatus 与表驱动测试
- [completed] 4. 对照 Silo 梳理 Bucket Policy classifier、handler、错误顺序、匿名访问与持久化接线清单
- [completed] 5. 集成审计、fresh PG17、workspace、Clippy/fmt/diff 回归
- [in_progress] 6. Bucket Policy 持久化/API/SigV4/anonymous 授权纵向接线

## 不变量

- quota 在 begin intent 时 reserve，commit 时只转移一次，abort/expire 时只释放一次；底层 Multipart parts 不重复收费。
- Enabled 保留的历史 data version 继续占用 used；仅逻辑永久删除 data version 才释放，delete marker 永远不计容量。
- Unversioned/Suspended null replacement 在同一事务处理新版本入账与旧 null data 释放；失败、重放和竞争不能漂移。
- Lifecycle 复用同一 quota 释放原语，不得建立第二套单边账本。
- Bucket Policy 必须显式 Deny 优先、默认拒绝、资源绑定目标 bucket；未知条件/operator 在 PUT 期拒绝。

## 错误记录

| 错误 | 尝试 | 处理 |
|---|---:|---|
| 并发代理派发文本内的反引号被 JavaScript 模板字符串误解析，调度脚本在执行前失败 | 1 | 未启动代理、未改源码；改用不含反引号的纯文本提示重新派发 |
| 配额聚合检索包含不存在的旧文件名 `s3_objects.rs`，导致并行包装器返回失败并丢弃其余输出 | 1 | 改用 `rg --files` 已确认的 `s3_object_service.rs`，后续聚合使用可独立返回结果的调用 |
| Windows PowerShell 未展开传给 `rg` 的 `crates/mediahub-server/src/s3*` 路径通配符 | 1 | 改为目录参数配合 `-g 's3*.rs'`，不再传路径 glob |
| 查看可选 cargo/rustc 进程时 `Get-Process` 无匹配返回非零，包装器将已有文件时间输出标为失败 | 1 | 已取得文件稳定时间；后续可选进程查询显式使用 `try/catch` 或省略，不重复该命令 |
| 内存配额测试首次使用 `--exact` 但未包含模块全名，两个命令均为 0 tests | 1 | 编译已通过；改用唯一测试名子串且去掉 `--exact` 重新执行 |
| 共享开发期间 Lifecycle contract 将尚未定稿的 `0014` 应用到临时库，迁移文件随后被 Policy 代理完善，SQLx 正确报告 migration 14 checksum changed | 1 | 不篡改迁移历史；在同一临时 PG17 创建新的独立数据库，从 0001 到定稿 0014 做 fresh migration 复验 |
| 检查尚未创建的并发输出文件时 `Get-ChildItem -ErrorAction SilentlyContinue` 仍使包装命令返回非零 | 1 | status 已正常取得；后续用 `Test-Path` 后再读取，不把“文件尚未产生”作为错误查询 |
| SigV4 审查沿用了不存在的 `s3_sigv4.rs` 文件名，实际实现位于 `s3_gateway.rs` | 1 | `rg` 已给出真实路径；改读 `s3_gateway.rs`，不重复旧路径 |
| 创建最终 fresh DB 时，对数据库不存在所产生的空 `psql` 输出直接调用 PowerShell `.Trim()` | 1 | 后续 `createdb` 和存在性查询均成功；后续检查不再对可能为空的输出调用实例方法 |
| 全工作区回归中旧 Object Tagging 合约让两个应用创建同名 Bucket，与 `0014` 的 S3 全局 Bucket namespace 冲突 | 1 | 保留跨应用版本隔离语义，但为另一应用使用不同的全局 Bucket 名称；这是测试假设更新，不放宽生产唯一约束 |
| Server 公共路径隔离测试同样依赖不同应用可创建同名 `assets` Bucket | 1 | 将另一应用测试桶改为 `other-assets` 并同步请求路径，继续验证应用路径隔离而不违反全局 Bucket namespace |
| 预签名 GET 集成测试的签名 helper 将绝对 URL 直接作为 SigV4 canonical URI，而生产验签使用 HTTP origin-form | 1 | helper 改为签署 `path_and_query`，并新增发送前本地 parse/verify 断言；生产验签器无需放宽 |
| 修正 canonical URI 后预签名 helper 本地自验仍失败：它仍按空 body 摘要签名，而验签器按 S3 预签名规则使用 `UNSIGNED-PAYLOAD` | 1 | query presign 明确使用 `SignableBody::UnsignedPayload`；保留本地自验和真实 HTTP 回归双重断言 |
| Workspace strict Clippy 报告 `S3BucketDeleteOperation` 三个变体重复 `Delete` 前缀 | 1 | 变体简化为 `Bucket`、`Policy`、`Lifecycle`，不添加 lint 豁免，协议行为不变 |
| 修复签名 helper 的首个补丁误把 `task_plan.md` 表格上下文放在 `tests.rs` 文件段内 | 1 | 拆分为带显式文件切换的补丁后成功应用，未产生部分源码改动 |
