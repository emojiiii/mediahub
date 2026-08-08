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
