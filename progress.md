# 研究进度

## 2026-08-07

- 已读取 `planning-with-files` 技能说明并建立三个研究记录文件。
- 已盘点 MediaHub 的 README、Rust workspace、前端配置、API 路由、能力开关、S3 版本拒绝逻辑、生命周期、Webhook、Variant 和测试证据。
- 已核对 pgsty/silo GitHub README、官方 Docs 导航、Silo/MinIO 兼容性审计、对象锁定和 bucket replication 文档。
- Git shallow clone 因 Schannel TLS 握手失败，改用 GitHub/官方 Docs 完成核对；没有留下 `.research` 克隆目录。
- 关键结论：Silo 的优势是通用 S3 基础设施；MediaHub 的优势是面向 AI 媒体产物的控制面、处理和体验。
- 关键实现差距：MediaHub 当前 `versionId` 未实现，capabilities 中 video processing、resumable upload、archive restore 为 false；Silo 具备分布式、纠删码、修复、复制、对象锁、企业 IAM、SSE 和多种交付面。
- 推荐路线：MediaHub 保持媒体控制面，优先补齐自身已声明但未启用的媒体能力；将 Silo 作为 S3 数据面/后端，而不是重写 Silo 的存储内核。

## 2026-08-08

- 已读取并执行 `imagegen` 与 `planning-with-files` 技能，接续现有规划文件。
- 已并行拆分官网、静态品牌、S3 Bucket API、S3 数据模型审计和跨层 Repository 任务。
- 已用内置 imagegen 生成两版 PrismArk 棱镜/方舟 Logo，并按技能要求转换为透明 PNG。
- 已生成 1024、512、192、64、32 尺寸和无损 WebP 品牌资产；透明角与 alpha 边界检查通过。
- 静态品牌代理已完成 PrismArk/万象仓、README、基础 SEO 元数据和 SVG Logo/Favicon，typecheck 通过。
- S3 默认假设：独立 listener 端口 9000、Host 不写死、Unversioned 覆盖版本 GC 宽限 24 小时且可配置。
- 已完成 PrismArk 响应式官网并接入公开根路由；控制台入口迁到 `/console`，登录与应用删除跳转同步调整。
- 官网加入唯一 H1、语义化结构、SoftwareApplication JSON-LD、路由级标题/描述、robots、manifest 和 PWA 图标。
- 官网与控制台均接入 PrismArk 正式 Logo；公开页加入亮色/暗色/跟随系统切换且使用 `prismark:theme` 持久化。
- 前端全量测试 33 文件/164 用例通过，TypeScript typecheck 通过。
- Playwright 桌面与 390×844 移动端快照通过；官网控制台 0 error/0 warning，深色主题与新存储键验证通过。
- `/console` 未登录跳转到 `/login` 正常；唯一错误是本地验收未启动 API 导致 `/api/v1/auth/me` 连接拒绝，非前端运行时错误。
- 已完成 S3 Bucket 基础协议：List/Create/Head/Delete/GetLocation，并接入 `/s3`、`/s3/` 与 Bucket PUT/DELETE 路由。
- 已建立正确的 S3 Bucket 三态版本控制与不可变 Object/ObjectVersion PostgreSQL 基础；对象最新版本仅由 `current_version_id` 推导。
- 已生成并接入 imagegen 正式 Logo，官网、控制台与 README 统一使用同一品牌资产。
- 已补运行时 canonical、Open Graph/Twitter URL 与图片、私有路由 noindex；技术 SEO 单测 8 个通过，TypeScript typecheck 通过。
- 官网最终门禁：33 文件/164 测试通过、类型检查通过、生产构建通过；浏览器桌面/移动端与明暗主题已验证。

### Server Multipart 收口（2026-08-08）

- CreateMultipartUpload 冻结 content_type/user_metadata，不再创建 Media 或保存 Visibility。
- UploadPart 使用流式临时写入并同时计算 SHA-256/MD5；part ETag 为 MD5，替换与失败清理由持久 GC task 接管。
- CompleteMultipartUpload 首次才流式 compose；attached UploadIntent takeover 会校验身份与冻结事实并复用 Ready/lease-expired Committing intent，不重复 compose。
- Multipart commit 复用普通 Put 的 canonical ObjectVersion commit builder，支持 null replacement、版本响应头与同版本/ETag 重放。
- giant S3 PostgreSQL 测试已改为 S3Object/current ObjectVersion payload、versionId=null/NoSuchVersion、List/Delete current-pointer 与持久 GC 断言。
- 验证：app 41/41、PostgreSQL adapter 23/23、Server Multipart 7/7；Server check、test --no-run、严格 no-deps Clippy、fmt 与 diff check 通过。
- 本阶段最初只完成编译验证；后续 Docker 真实数据库结果见“Docker 真实后端复验”。

### 最终复验与文档收口（2026-08-08）

- PostgreSQL adapter 库测试 23/23 通过；旧 Media-based Multipart 契约已删除，`repository_contract --no-run` 已重新可编译。
- Server S3 HTTP 测试 40/40、Multipart 7/7、邮件模板 2/2 通过；Server 全量测试目标 `--no-run` 通过。
- Server、App、PostgreSQL adapter 严格 `--no-deps` Clippy 通过；`cargo fmt --check`、`git diff --check` 通过。
- `docker compose config --quiet` 使用仅限当前校验进程的测试密钥通过，没有写入 `.env`。
- Docker Desktop 最初未运行；用户启动后已继续完成真实后端复验，结果见下一节。
- README、S3 修改文档与 Runbook 已同步核心闭环和真实剩余差距；邮件、WebDAV Realm、更新 User-Agent 已统一为 PrismArk。

### Docker 真实后端复验（2026-08-08）

- 使用无持久卷、随机本机端口的 PostgreSQL 17 临时容器执行 fresh migration。
- 修复 `0012` 中两个自动/显式 state constraint 同名问题，以及 PostgreSQL `chr(0)` 检查导致所有对象 INSERT 失败的问题。
- PostgreSQL Repository Contract 1/1 真实通过；Server 全量数据库与非数据库测试 133/133，Server lib 8/8 通过。
- 使用官方 `docker.io/pgsty/silo:latest` 临时容器和独立 bucket 运行真实 S3 Adapter 合同。
- 修复 S3 backend 不支持默认 `copy_if_not_exists` 的问题：条件 Multipart Copy 保证不覆盖，后续同源 Copy 恢复 Content-Type/checksum metadata，并进行幂等校验。
- Silo ObjectStore 合同与真实 Presigned PUT 1/1 通过；S3 Adapter 单元测试 8/8、严格 Clippy 通过。
# S3 Listing vertical slice progress (2026-08-08)

- Confirmed clean worktree and exact baseline `bdd6323`.
- Read planning skill and existing planning context.
- Began audit of existing repository/list parser/XML/classifier boundaries.
- Confirmed the bucket-scoped listing APIs require new repository DTOs rather than reusing the per-object audit lookup.
- Identified existing limit+1 SQL/list and XML escaping patterns to reuse.
- Defined target marker semantics: key-only markers skip the marked key; paired markers resume within the marked key, then continue to later keys.
- Chose active Multipart states `pending` and `completing`; terminal/expired uploads are not protocol-visible.
- Completed repository/classifier/schema audit and started app/SQL implementation.
- Added app listing queries/items/pages and `S3ListingRepository`.
- Added parameterized PostgreSQL version and multipart listing queries; `cargo check -p mediahub-adapter-postgres` passes.
- Added XML result DTOs/renderers for version and multipart listings.
- One import patch missed current CopyObject context; no source was changed by that failed patch, and the next patch preserves those imports.
- Added `?versions`/`?uploads` classifier branches and bucket dispatch.
- Added strict query parsing and handlers for both listing APIs.
- Added S3 XML renderers for versions/delete markers and multipart uploads/common prefixes.
- `cargo check -p mediahub-server --bin mediahub-server` passes.
- Added and compiled a real PostgreSQL listing contract test; Docker image startup timed out and is being inspected without retrying the same command blindly.
- Real PostgreSQL listing contract passed 1/1 and the disposable container was removed.
- Workspace strict clippy reached concurrent CopyObject code and stopped on two unrelated `collapsible_if` warnings; listing files had no reported lint before that boundary.
- Final `cargo fmt --all -- --check`, server check, focused HTTP tests, PostgreSQL tests, and `git diff --check` pass.
- Strict app/PostgreSQL clippy passes; server passes with only the two concurrent CopyObject `collapsible_if` findings explicitly allowed.
- Verified `s3_http_listing_tests.rs` exists on disk. No commit or push was performed.

## 第二阶段并行实现进度（2026-08-08）

- 已创建本地提交 `bdd6323`，确认未 push。
- 已成功克隆 Silo HEAD `100e2e5` 到 `.research/silo`，根 `.gitignore` 排除该研究目录。
- 已完成 CopyObject / UploadPartCopy，包含 source version、metadata directive、条件头、Range part copy、标准 XML 与版本响应头。
- 已完成 ListObjectVersions / ListMultipartUploads，全部从 PostgreSQL metadata 分页，不扫描对象存储。
- 已把 WebDAV GET/HEAD/PUT/DELETE/COPY 迁移到 ObjectVersion；MOVE 因缺少原子 exact-source delete fence 返回明确 501。
- 已完成 Bucket Object Lock GET/PUT/CreateBucket header、不可逆启用与 Versioning 原子约束。
- 已完成不可变 ObjectVersion Preview Manifest 与 GET/HEAD Content，支持 Application 隔离、ETag、Range、404 隐藏和安全响应头。
- 已新增 native-client 兼容脚本；Copy/List 四项从旧 XFAIL 改成严格 PASS，报告基线改为 Git SHA + dirty 状态。
- 已完成 Win11 瀑布流真实图片缩略图：视口前后 240px 懒加载、最多 4 并发、Query 缓存、失败回退；栅格图使用 384×384 WebP Q68 Variant。
- PostgreSQL 17 统一回归：Server 155/155、listing contract 1/1、Bucket Object Lock contract 1/1。
- pgsty/silo 真实 ObjectStore 合同 1/1 通过；临时 Silo 容器和 bucket 已删除。
- 对象级 Retention/Legal Hold、PutObject 锁头、默认 Retention、签名 Governance bypass 和真实 SigV4 往返已完成；PG 合同 1/1、Server 160/160。
- 栅格图片预览已支持 10%–800% 缩放、适应窗口、100%、拖拽、双击与键盘控制；前端全量 179/179、构建通过。
- 最终 Rust 全工作区、全部 targets 在 PostgreSQL 17 上通过；Server 160/160，Repository/Listing/Bucket Lock/ObjectVersion Lock 合同全部通过。
- 全工作区严格 Clippy `-D warnings`、`cargo fmt --check`、OpenAPI 10/10 与生成客户端一致性、Compose 配置、PowerShell 兼容脚本语法/Help 均通过。
- 最终 PostgreSQL 与 Silo 临时容器均已按精确名称删除，没有测试容器残留。

## S3 Object Tagging 纵向切片（2026-08-08）

- 已确认干净基线 `72eb4d4`，且不会 commit/push。
- 已读取 `planning-with-files` 技能与现有规划记录。
- 已定位 Silo 对象 tagging 路由顺序，以及 PrismArk 当前 CopyObject 的显式拒绝边界。
- 首次检索误用根级 `migrations/` 路径、首次计划补丁使用乱码上下文；均已记录并更换方法。
- 已确定独立标签表与事务边界：初始标签进入版本 commit，后续替换由版本锁定 repository 命令完成。
- Windows 下 `rg crates/.../s3_http*.rs` 不展开通配符导致一次检索失败；后续改用 `rg -g 's3_http*.rs'`。
- 已完成 Core/App/Memory/PostgreSQL/HTTP/XML 全纵向实现；对象标签绑定精确 ObjectVersion，并覆盖 current/versionId、应用隔离与 delete marker 拒绝。
- 已完成 PutObject 标签、CopyObject COPY/REPLACE、Multipart Initiate 冻结、GET/HEAD 标签计数；所有不支持组合均显式报错。
- 标签字符校验已拒绝全部 control，新增换行与 Tab 负测；XML namespace/顺序和 header URL 解码均严格校验。
- Core 54/54、App 45/45、PostgreSQL adapter 28/28、Server 165/165 已通过。
- 使用 PostgreSQL 17 tmpfs 临时容器完成真实 tagging contract 1/1 与最终 SigV4 HTTP 1/1；容器已删除且无卷残留。
- 全工作区 strict clippy、fmt check 与 diff check 通过；HEAD 仍为 `72eb4d4`，未 commit、未 push。

## 第三阶段并行收口（2026-08-08）

- 已完成 Object Lock AWS CLI 矩阵：Bucket 配置、默认 Governance、Retention/Legal Hold、无 bypass/未签名 bypass 拒绝、签名 bypass 与精确安全清理。
- 已完成 Object Tagging AWS CLI 矩阵：精确版本 GET/PUT/DELETE、版本隔离、Copy COPY/REPLACE、TagCount 与本轮 VersionId 清理；无法由 CLI 原样发出的负例明确 SKIP。
- 对象预览加入上一项/下一项、当前位置与左右方向键；切换对象会隔离签名 URL、Variant、缩放和错误状态，输入控件与 Variant 面板不被方向键劫持。
- 主代理统一回归：Rust 全工作区全部 targets 通过，Server 165/165，PG Repository/Listing/Bucket Lock/ObjectVersion Lock/Object Tagging 合同各 1/1；严格 Clippy/Fmt 通过。
- 前端 34 个测试文件、182/182 通过，OpenAPI 50 paths/74 operations/491 references、类型检查、生产构建与 viewer chunk 验证通过。

## 标准 S3 Lifecycle 执行器纵向切片（2026-08-08）

- 已运行 planning session catchup，并确认干净基线 `2d512fd`，未 commit、未 push。
- 已创建本切片计划，开始只读审计 worker、Lifecycle 配置、对象删除、GC、Object Lock 与 Multipart 清理边界。
- 初始检索确认 Core 已有 Lifecycle 配置模型、ObjectVersion noncurrent 时间、Lifecycle GC reason；尚无标准对象 Lifecycle worker。
- 已确认现有 worker 只处理 legacy Media lifecycle 和全局 Multipart TTL；标准 S3 Lifecycle 将作为独立有界 pass 接入，并继续由持久 GC worker 删除底层 blob。
- 已确认 Bucket S3 configuration revision 可直接用作执行 fence，Multipart 也已有事务化 abort+GC 清理原语。
- 已完成第一阶段事务审计：确定 Lifecycle 采用“有界 scan candidate → 专用 transaction execute → 既有持久 GC worker”三段式，不直接调用外部 DeleteObject 命令。
- 已开始定义 repository DTO：current candidate 带 expected current/version/config revision，exact candidate 带内部 version id/noncurrent timestamp，multipart candidate 带 upload id/initiation/config revision。
- 一次只读 helper 检索因 PowerShell 文件通配符未展开而失败，已记录并切换为 `rg -g`；未产生源码修改。
- 第二次只读聚合被 `rg` 的正常“无匹配”退出码中止，已记录；改为确定文件读取后直接进入实现。
- 已新增可编译的 App 生命周期端口、候选/命令/outcome、UTC 日边界 helper 和有界 `S3LifecycleService`；`cargo check -p mediahub-app` 通过。
- Service 已支持 Enabled+Prefix、current Days/Date、noncurrent Days、expired marker、multipart abort、跨规则去重、配置 revision command fence 与 bucket cursor。
- 已补齐 Memory Lifecycle repository、真实 lifecycle configuration revision 更新、对象/版本执行与 LifecycleExpiration GC；首次 test-target 编译只发现一个缺失 import，已修复。
- 已完成 PostgreSQL metadata-only 候选查询、同事务配置/current/exact/Object Lock 重检、LifecycleExpiration GC、expired marker 与 Multipart abort；`cargo check -p mediahub-adapter-postgres --tests` 一次通过。
- 新增 `0013_s3_lifecycle_executor.sql` 元数据索引；未扫描对象存储。
- Server worker 已接入有界 S3 Lifecycle batch 与内存 cursor；首次补丁误命中后方 match，已按精确行段移动到主循环。
- 可编译完整纵向切片已交付：App、PostgreSQL `--tests` 和 Server binary 均编译通过；现在进入 Fake Clock、表驱动与真实 PG17 竞争合同阶段。
- 已新增 UTC 日边界表驱动测试和 Fake Clock 有界 batch 测试；首次编译只缺测试类型 import，已修复。
- App Lifecycle 针对性测试 2/2 通过；剩余一个 Memory multipart seed dead-code warning 将由 Memory 执行测试消除。
- Memory Lifecycle current expiration + Multipart cleanup 端到端测试 1/1 通过，dead-code warning 已消除。
- 新增真实 PostgreSQL contract，`--no-run` 编译通过；临时 PG17 使用 tmpfs 与随机回环端口 `127.0.0.1:49415`，容器名 `prismark-lifecycle-pg17-codex`。
- 真实 PostgreSQL 17 Lifecycle contract 1/1 通过，覆盖 current/noncurrent/sole marker/Multipart/idempotency/config revision/current head/Object Lock 延长。
- 修正 batch 在“恰好耗尽 limit”时的 bucket cursor：保留当前 bucket 以便下一批继续剩余 action，避免恰逢满页时跳过该 bucket 后续规则。
- App 全量 48/48、PG17 Lifecycle contract 1/1、Core Lifecycle 3/3 通过。
- PostgreSQL lib 27/28 通过；唯一失败是旧源码静态断言未跟随共享 Object Lock helper 重构，待更新后重跑。Server lifecycle filter 因失败即停尚未执行。
- PostgreSQL lib 修复后 28/28 通过；Server S3 Lifecycle parser/schema 7/7 通过。宽泛过滤另命中一个缺 `DATABASE_URL` 的无关认证测试，已改为精确过滤策略。
- 精确 Server Lifecycle parser/schema 7/7 通过，transition/tag filter 等不支持能力仍在 PUT 阶段显式拒绝。
- 首次 fmt check 仅有标准排版差异，准备运行 workspace formatter。
- `cargo fmt --all` 已执行，随后 `cargo fmt --all -- --check` 通过。
- 全工作区 strict Clippy `cargo clippy --workspace --all-targets -- -D warnings` 通过。
- Worker/Fake Clock/Memory/PG17/配置竞争/current race/Object Lock 延长测试阶段完成，进入全工作区串行回归与最终清理。
- 全工作区 `cargo test --workspace --all-targets -- --test-threads=1` 通过：Server 165/165、App 48/48、Core 54/54、PG adapter 28/28，全部 PG contracts（含 Lifecycle 1/1）通过；真实外部 S3 contract 按既有条件保持 ignored。
- `git diff --check` 通过。范围守卫检测到任务期间由并发工作新增的兼容文档/脚本改动；本切片未触碰，也不会回退或纳入 Lifecycle 修改清单。
- 新增 Unversioned/Suspended Lifecycle delete 语义测试；首次运行因旧测试 helper 缺新内容的 MD5 向量而失败，已改用既有固定向量。
- 补充后 App 全量 49/49 通过，明确覆盖 Date due/future、Unversioned 与 Suspended 生命周期删除语义。
- 最终 workspace strict Clippy、fmt check、`git diff --check` 再次通过。
- 临时 PostgreSQL 17 容器 `prismark-lifecycle-pg17-codex` 已按精确名称删除；使用 tmpfs，未创建匹配卷。
- 终态审阅补齐 Memory stale-completing attached-intent 清理；Memory Lifecycle 2/2、App Lifecycle planner 2/2、App strict Clippy 通过。
- 最终 App 全量 49/49、workspace strict Clippy、fmt check、`git diff --check` 通过；没有重新创建测试容器。

## PostgreSQL Lifecycle 事务安全与资源边界修复（2026-08-08）

- 已确认 S3 Put、Multipart commit 与 S3 delete 均未维护 `applications.used_bytes/reserved_bytes`；依用户补充约束，本轮不单边修改 quota，完整 S3 quota 记为独立下一切片。
- 已把 Multipart Lifecycle 执行改为 `upload FOR UPDATE -> bucket FOR SHARE -> revision/rule recheck -> abort/cleanup`，与 CompleteMultipartUpload 的 upload-first 顺序一致，消除已识别 ABBA。
- 已移除 Lifecycle 的全历史 `fetch_all + FOR UPDATE`；current/noncurrent/null/marker 均按 ID 或唯一 null version 精确锁，EODM 在 object head 锁下用存在性查询确认没有 sibling。
- PostgreSQL adapter test target 已通过编译；首次编译发现 SQL identifier 转义与 owned version 借用问题，已定向修复。
- 0013 已新增 lifecycle bucket partial index，并按 tenant/state/order 调整 current/noncurrent/marker/multipart 索引；候选 prefix 改用转义 LIKE，避免 `LEFT(...)` 阻断 pattern index。
- 一次性 PostgreSQL 17 fresh migration 与真实 `s3_lifecycle_contract` 1/1 通过；竞争合同确认 Complete-like 事务持有 upload 时仍可立即取得 bucket，Lifecycle 随后恢复并完成 abort。
- 精确锁合同在无关 active sibling 被另一个事务 `FOR UPDATE` 持有期间仍完成目标 noncurrent 删除；特殊 `%`、`_`、反斜杠 prefix 合同通过；quota sentinel 在全部动作和幂等重跑后保持不变。
- 根据集成反馈，PG marker 创建时间、前版本 noncurrent 时间和 object 更新时间已统一为 UTC 当日 00:00；eligibility/Object Lock 和永久删除仍使用实际 evaluated_at。
- PG 已显式 override 普通 Expiration marker 候选：Days 下推 marker cutoff，Date 到期前返回空，显式 EODM 即时候选；执行期锁 marker 后用 `_at` helper 二次复检。
- Multipart helper 已拆出 attached intent prelock 与“使用已锁 intent 清理”原语；Lifecycle 现在严格按 `upload -> intent -> bucket -> cleanup`，并新增 intent/bucket 竞争合同。
- Prefix 进一步从转义 LIKE 改为 Unicode scalar 安全的 C-collation 半开区间；0013 object prefix index 改用默认 C 排序 opclass并 include current pointer。真实 EXPLAIN 的 current/marker 计划已无 LIMIT 前 Sort，noncurrent/multipart 也沿时间顺序 partial index。
- 最终门禁通过：App Lifecycle 4/4、PG adapter lib 28/28、fresh PG17 Lifecycle contract 1/1、Server binary check、adapter all-target strict Clippy、workspace fmt check、git diff check。
- 测试 PostgreSQL 17 容器采用 tmpfs 且无 volume，已按精确名称删除；未 commit、未 push。
- 主代理终态语义审计修正 UTC Days cutoff 为 `start_today - days` 且严格 `<`；并修正 Enabled current Expiration，使受保护 data version 仍可创建 delete marker，受保护 noncurrent exact version 才返回 `Locked`。
- 独立只读并发审计发现并阻止了提前提交：Lifecycle Abort/Multipart Complete 存在反向锁序，planner 的空扫描不消耗预算且会饥饿后续 action，PostgreSQL target 会无界锁定单 key 全部活跃版本。
- 审计同时确认普通 Days/Date Expiration 还应清理到期 sole delete marker，Lifecycle marker 时间应归一到动作日 UTC 00:00；这些问题已拆为两个不重叠子任务并发修复。
- Quota 检查确认当前 S3 ObjectVersion 上传/提交整体尚未完整接入 `applications` 计量；本轮禁止只在 Lifecycle 侧单边扣减，完整 S3 quota 将作为独立纵向切片处理。
- 主代理使用全新 `postgres:17-alpine` tmpfs 容器独立复验：Lifecycle contract 1/1 通过；全 workspace 所有 targets 通过，App 52/52、Server 165/165、PG adapter 28/28，全部 PG contracts 各 1/1。
- 主代理再次完成 `cargo clippy --workspace --all-targets -- -D warnings`、workspace fmt check、raw SigV4 offline golden 与 `git diff --check`；仅有既有 Windows linker 信息提示。
- 主代理创建的容器 `prismark-lifecycle-pg17-main-0808b` 已按精确名称删除；使用 tmpfs、无匹配 volume。
- 已创建本地提交 `f004443 feat: execute S3 lifecycle and harden compatibility`，分支相对 origin ahead 4；未 push。
- 下一批已并发启动：完整 S3 Application quota、Bucket Policy Core evaluator、Silo Policy 协议/模块/测试只读对照；主代理负责事务锁序、错误映射和后续 API 集成审查。
- 主代理独立完成 S3 quota 静态审查：Put/Copy/WebDAV 复用 begin_put reservation，Multipart completion 复用 attached intent；终态 replay 不二次释放，delete marker 不计容量，null replacement 在单事务转移 new-old，未发现 application→bucket 反向锁。
- 内存 quota/reservation/replay 与 Suspended null replacement 针对性测试各 1/1 通过；PostgreSQL adapter tests 编译通过。
- 主代理启动 tmpfs、无 volume 的 PostgreSQL 17 容器 `prismark-s3quota-pg17-main-0808`（随机回环端口 64727）；真实 `s3_quota_contract` 1/1 与 quota 接入后的 `s3_lifecycle_contract` 1/1 均通过。
- Core Bucket Policy 已完成 strict parser/evaluator、稳定 JSON 与保守 PolicyStatus；主代理复验 Core 75/75 和 strict Clippy 通过。
- Bucket Policy persistence 已完成 0014、稳定 12 位账户 ID、全局 bucket 名、JSONB/hash/revision 与 tenant fence。第一次主线复验命中并发中间版 0014 的 SQLx checksum 保护；改用全新独立数据库从 0001 fresh migrate 后 contract 1/1 通过，未篡改迁移记录。
- 新一轮并发已启动：Server Bucket Policy/PolicyStatus HTTP 管理接口，以及 App 统一 resource-policy 授权决策服务；主代理负责 fresh PG、协议顺序、全局 bucket 冲突语义与最终集成。
- App 统一 S3 authorization service 已完成并交接：签名 principal 与 anonymous 显式分型，identity/bucket Deny 优先，同账户 Allow 并集、跨账户双 Allow、条件传递和 invalid persisted policy fail-closed；App 63/63、strict Clippy 通过。
- AWS 官方文档复核后已纠正 HTTP 子任务：GetBucketPolicyStatus 必须返回 XML，不是 SDK 展示的 JSON；PUT 采用 200 empty，DELETE 采用 204，跨 owner 405、expected owner mismatch 403。
- 标准 Access Key Identity Policy Core 切片已并发启动；目标是不使用旧 permissions 作为 S3 授权 fallback。
- Bucket Policy HTTP 管理切片已交接：GET/PUT/DELETE policy、GET policyStatus XML、20 KiB/Content-Length/MD5/stable JSON、expected owner、跨 owner 405、全局 CreateBucket 冲突均完成；focused + 真实 PG17 SQLx HTTP 7/7 通过。
- Raw SigV4 offline golden 再次通过；HTTP 最终 strict Clippy/全量测试等待并发 Identity Policy Core 文件从中间态稳定后统一执行。
- Identity Policy Core 已完成：显式 Account/Bucket/Object 资源域、identity-only actions、Principal 禁止、Deny 优先、稳定 JSON 与严格限制；Core 80/80、App 63/63。
- 主线使用全新 PostgreSQL 17 数据库完成 workspace all-targets 串行回归：PG quota/policy/lifecycle/tagging/lock/listing contracts 全部通过，Server 172/172、Core 80/80、App 63/63；仅真实外部 S3 contract 按环境约定 ignored。
- 全量回归修正了两个全局 Bucket namespace 的旧测试假设，并修复集成测试 SigV4 presigner 的 origin-form 与 `UNSIGNED-PAYLOAD` 构造；生产验签未放宽，预签名 GET 现在发送前自验与真实 HTTP 双通过。
- Workspace strict Clippy `-D warnings`、workspace fmt 已通过；当前切片准备本地提交，不 push。下一切片继续 Identity Policy persistence/Server decision 与 anonymous 数据面授权接线。
