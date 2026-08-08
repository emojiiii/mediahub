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
