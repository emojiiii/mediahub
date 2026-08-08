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
- Policy、完整 Lifecycle 执行器、Notification/CORS/SSE 等未在本切片完成的能力必须明确列为剩余差距，不伪装为已支持。

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
