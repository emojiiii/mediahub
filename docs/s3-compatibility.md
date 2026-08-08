# PrismArk S3 客户端兼容矩阵

本文描述 PrismArk 当前检出版本的可重复 S3 客户端兼容测试。矩阵面向 Windows PowerShell 5.1+ 与 Docker Desktop，检测本机已有的 AWS CLI、mcli/mc 和 rclone；脚本不会下载、安装或升级任何客户端。报告中的 `baseline` 会自动记录当前 Git 短提交号，工作区存在未提交改动时附加 `+dirty`，避免把新能力误归到旧提交。

主脚本：

```powershell
scripts/s3-compat/Invoke-PrismArkS3Compatibility.ps1
```

## 运行目标

脚本支持三种目标：

| Target | 启动内容 | PrismArk 物理存储 | 适用场景 |
| --- | --- | --- | --- |
| `DockerLocal` | 临时 PostgreSQL + 临时 PrismArk | PrismArk 容器文件系统 | 默认、本地快速验证 |
| `DockerSilo` | 临时 PostgreSQL + 临时 pgsty/silo + 临时 PrismArk | Silo 容器文件系统 | 验证 PrismArk → Silo 的完整链路 |
| `Endpoint` | 不启动容器 | 由目标环境决定 | 验证已运行的 PrismArk S3 endpoint |

Docker 模式不调用仓库现有 `docker-compose.yml`，不声明或挂载 volume，也不会执行 `docker compose down -v`。每次运行都会创建带随机 ID 的独立 Docker network 和 `--rm` 容器，并在 `finally` 清理容器、network 和系统临时目录。已有 Compose 容器和 volume 不在脚本的目标集合内。

默认会从当前工作区构建 `prismark-s3-compat:local`。传入 `-SkipBuild` 时，脚本只使用 `-PrismArkImage` 指定的现有镜像。

## 前置条件

- Windows PowerShell 5.1 或 PowerShell 7。
- `DockerLocal` / `DockerSilo` 需要已启动 Docker Desktop。
- 至少安装一个待测客户端，且命令位于 `PATH`：
  - `aws`
  - `mcli`，或旧命令名 `mc`
  - `rclone`
- `DockerSilo` 默认使用 `docker.io/pgsty/silo:latest`；可通过 `-SiloImage` 固定版本或 digest。

客户端缺失不是矩阵失败。脚本会为该客户端及其用例写入明确的 `SKIP`，不会自动安装，也不会用另一种客户端冒充它。

## 运行方式

### 临时 PrismArk + 本地存储

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File `
  .\scripts\s3-compat\Invoke-PrismArkS3Compatibility.ps1 `
  -Target DockerLocal
```

### 临时 PrismArk + Silo 存储

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File `
  .\scripts\s3-compat\Invoke-PrismArkS3Compatibility.ps1 `
  -Target DockerSilo `
  -SiloImage docker.io/pgsty/silo:latest
```

### 已有 PrismArk endpoint

Endpoint 模式只从进程环境读取密钥。不要把 Secret Access Key 写入命令行参数、仓库文件或报告目录。

```powershell
$env:PRISMARK_S3_ACCESS_KEY_ID = '<application-access-key-id>'
$env:PRISMARK_S3_SECRET_ACCESS_KEY = '<one-time-secret-access-key>'

powershell -NoProfile -ExecutionPolicy Bypass -File `
  .\scripts\s3-compat\Invoke-PrismArkS3Compatibility.ps1 `
  -Target Endpoint `
  -Endpoint http://127.0.0.1:9000 `
  -Region us-east-1
```

运行结束后可删除当前终端中的变量：

```powershell
Remove-Item Env:PRISMARK_S3_ACCESS_KEY_ID -ErrorAction SilentlyContinue
Remove-Item Env:PRISMARK_S3_SECRET_ACCESS_KEY -ErrorAction SilentlyContinue
```

### 使用已有 PrismArk 镜像

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File `
  .\scripts\s3-compat\Invoke-PrismArkS3Compatibility.ps1 `
  -Target DockerLocal `
  -PrismArkImage ghcr.io/example/prismark:test `
  -SkipBuild
```

### 帮助与自定义报告目录

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File `
  .\scripts\s3-compat\Invoke-PrismArkS3Compatibility.ps1 -Help

powershell -NoProfile -ExecutionPolicy Bypass -File `
  .\scripts\s3-compat\Invoke-PrismArkS3Compatibility.ps1 `
  -OutputDirectory C:\temp\prismark-s3-results
```

Raw SigV4 helper 的离线 golden、endpoint 防护、AST 安全规则、精确版本清理规则和 operation 同步可以独立验证，不需要 AWS CLI、Docker 或网络：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File `
  .\scripts\s3-compat\Test-S3Compat.RawSigV4.ps1
```

## 参数与环境变量

| 名称 | 类型 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `-Target` | 参数 | `DockerLocal` | `DockerLocal`、`DockerSilo` 或 `Endpoint` |
| `-Endpoint` | 参数 | 空 | `Endpoint` 模式必填；应为 PrismArk S3 origin |
| `-Region` | 参数 | `us-east-1` | SigV4 region |
| `-PrismArkImage` | 参数 | `prismark-s3-compat:local` | 构建标签或 `-SkipBuild` 使用的镜像 |
| `-SiloImage` | 参数 | `docker.io/pgsty/silo:latest` | `DockerSilo` 使用的镜像；正式 CI 建议固定 digest |
| `-PostgresImage` | 参数 | `postgres:17-bookworm` | 临时 PostgreSQL 镜像 |
| `-SkipBuild` | 开关 | false | 不构建工作区，要求 PrismArk 镜像已存在 |
| `-OutputDirectory` | 参数 | `scripts/s3-compat/results` | JSON、Markdown 输出目录 |
| `-TimeoutSeconds` | 参数 | `180` | 单个环境 readiness 的最长等待时间，范围 30–900 秒 |
| `PRISMARK_S3_ACCESS_KEY_ID` | 环境变量 | 无 | `Endpoint` 模式必填 |
| `PRISMARK_S3_SECRET_ACCESS_KEY` | 环境变量 | 无 | `Endpoint` 模式必填；永不写入报告 |

Docker 模式运行时生成 PostgreSQL 密码、AccessKey 主密钥、媒体签名密钥、测试账户密码和 Silo root 密钥。它们只保留在当前 PowerShell 进程环境与临时容器环境中，运行后恢复原环境变量。临时 PrismArk 通过仅限隔离环境的 `MEDIAHUB_EXPOSE_AUTH_TOKENS=true` 完成注册、邮箱验证、登录和 Application AccessKey 创建。

## 状态语义与退出码

| 状态 | 定义 | 是否导致非零退出码 |
| --- | --- | --- |
| `PASS` | 命令成功，并且响应、对象内容或状态断言通过 | 否 |
| `SKIP` | 客户端不存在，或该客户端没有稳定、可观察的命令表达此协议操作 | 否 |
| `XFAIL` | PrismArk 返回明确的已知未实现协议错误，例如 `NotImplemented`、`Unsupported` 或 HTTP 501 | 否 |
| `FAIL` | 非预期错误、认证/连接失败、响应格式错误、状态错误、内容不一致，或接口返回成功但缺少应有资源 | 是 |

矩阵只在至少存在一个 `FAIL` 时返回退出码 1。只有 `PASS`、`SKIP`、`XFAIL` 时返回 0。

`XFAIL` 不是“忽略任何错误”。预期缺口探测只接受明确的未实现错误；超时、签名失败、500、无效 JSON 等仍为 `FAIL`。如果原本未实现的接口开始成功，脚本会继续验证实际资源：验证通过记为 `PASS`，200 空响应或错误资源记为 `FAIL`。

## 覆盖范围

AWS CLI 提供最完整的低级 S3 API 覆盖：

- Bucket：CreateBucket、HeadBucket、ListBuckets、DeleteBucket。
- Object：PutObject、GetObject、HeadObject、单 Range、If-Match、If-None-Match。
- ListObjectsV2：prefix、delimiter、CommonPrefixes、max-keys、continuation-token。
- DeleteObjects：多对象请求与 Deleted 结果。
- Versioning：`null` version、Enabled、opaque VersionId、delete marker、精确版本读取与删除。
- Multipart：CreateMultipartUpload、UploadPart、ListParts、CompleteMultipartUpload、AbortMultipartUpload。
- 能力探测：CopyObject、UploadPartCopy、ListObjectVersions、ListMultipartUploads。
- Object Tagging：PutObject `--tagging`、精确 VersionId 的 Get/Put/DeleteObjectTagging、版本隔离、TagCount、CopyObject 默认 COPY 与显式 REPLACE，以及由内置 Raw SigV4 发包器执行的四类服务端负例。
- Object Lock：创建时启用 Object Lock、GET/PUT bucket configuration、默认 GOVERNANCE retention、PutObject 显式 retention/legal hold、精确 VersionId 的 GetObjectRetention/GetObjectLegalHold、拒绝无 bypass 与未签名 bypass、接受已签名 governance bypass。

mcli/mc 覆盖 bucket、对象上传下载/stat、Range、高级目录列表、删除和 Versioning enable。mcli 没有稳定低级入口的条件请求、显式 pagination、DeleteObjects、UploadPartCopy 及低级 Multipart 操作会记为 `SKIP`。

rclone 覆盖 bucket、对象上传下载/metadata、Range、prefix/delimiter、通过 `list_chunk=1` 强制的分页、批量删除和通过 5 MiB cutoff 强制的自动 Multipart。它没有直接表达 Versioning、UploadPartCopy、ListParts、AbortMultipartUpload 的低级命令，因此这些项记为 `SKIP`。

## 防止“假成功”的能力探测

以下能力不会只根据进程退出码判断：

| 能力 | 成功后的附加断言 | 基线预期 |
| --- | --- | --- |
| CopyObject | Head 目标对象，并核对 Content-Length | 必须 `PASS`；失败或伪成功均为 `FAIL` |
| UploadPartCopy | Complete Multipart、下载目标并核对 SHA-256 | 必须 `PASS`；失败或内容不一致均为 `FAIL` |
| ListObjectVersions | 列表必须包含本轮已知的普通 VersionId 和 Multipart VersionId | 必须 `PASS`；失败或错误列表均为 `FAIL` |
| ListMultipartUploads | 列表必须包含本轮已知的 pending UploadId | 必须 `PASS`；失败或遗漏上传均为 `FAIL` |
| Put/GetObjectTagging | PutObject 必须返回非 `null` VersionId；GetObjectTagging 必须报告同一 VersionId，且键值集合完全相等 | 必须 `PASS`；缺字段、多标签、少标签、重复键或值不一致均为 `FAIL` |
| Put/DeleteObjectTagging | 替换后精确回读且不能保留旧标签；删除后必须返回空 TagSet，并确认另一版本标签不变 | 必须 `PASS`；合并旧标签或跨版本污染均为 `FAIL` |
| Object Tagging version isolation | 同 key 两个 VersionId 分别精确回读不同标签集合 | 必须 `PASS`；只验证 latest 或两个版本返回同一集合均为 `FAIL` |
| CopyObject Tagging | 默认 COPY 必须继承精确源版本标签；REPLACE 必须只保留 `--tagging` 指定集合；目标均使用返回的精确 VersionId 回读 | 必须 `PASS`；静默丢标签、错误继承或错误替换均为 `FAIL` |
| Head/Get TagCount | 对 AWS CLI 实际输出中存在的 TagCount 逐个核对；两个输出模型均不暴露时单独记为 `SKIP` | 暴露但计数错误为 `FAIL`；不将字段缺失伪装成 `PASS` |
| Tagging invalid key / duplicate key / too many tags | 对三个唯一 key 先创建并登记精确对象版本，再发送结构严格、`Content-MD5` 正确的 Raw SigV4 PutObjectTagging XML | 三项都必须返回 4xx、`Content-Type: application/xml`、`<Error><Code>InvalidTag</Code>`、非空 Message/RequestId，且响应头与 XML RequestId 一致；否则为 `FAIL` |
| Tagging bad percent encoding | 对唯一 key 发送带原始 `x-amz-tagging: bad=%` 的 Raw SigV4 PutObject | 必须返回同样可审计的 4xx S3 XML `InvalidTag`；2xx、其他错误码、超限响应或无法解析的 XML 均为 `FAIL` |
| Bucket Object Lock | Create 后核对 HeadBucket 与 Versioning=Enabled；PUT 默认规则后再次 GET 并核对 Enabled/GOVERNANCE/Days=1 | 必须 `PASS`；空配置、错误模式或未持久化均为 `FAIL` |
| Object retention / legal hold | PutObject 必须返回非 `null` VersionId 和 ETag；随后对同一 VersionId 核对 HeadObject、GetObjectRetention 与 GetObjectLegalHold | 必须 `PASS`；只返回成功码但状态不一致为 `FAIL` |
| Governance bypass | 无 bypass 与 `--no-sign-request` bypass 都必须失败且精确版本仍存在；正常签名 bypass 后精确版本必须 404 | 必须 `PASS`；错误地删除或伪成功均为 `FAIL` |

因此，错误地返回 200、空 XML、错误分页或错误对象不会被记录为通过。

## 报告

默认输出到被 Git 忽略的 `scripts/s3-compat/results/`：

```text
s3-compat-YYYYMMDD-HHMMSS-<run-id>.json
s3-compat-YYYYMMDD-HHMMSS-<run-id>.md
```

JSON 的 `schema_version` 当前为 1，主要字段包括：

```json
{
  "schema_version": 1,
  "run_id": "...",
  "baseline": "<git-short-sha>+dirty",
  "target": {
    "mode": "DockerLocal",
    "endpoint": "http://127.0.0.1:<dynamic-port>",
    "region": "us-east-1",
    "storage_backend": "local"
  },
  "clients": [],
  "summary": {
    "PASS": 0,
    "SKIP": 0,
    "XFAIL": 0,
    "FAIL": 0
  },
  "results": []
}
```

每个 result 包含 `client`、`operation`、`status`、`duration_ms` 和经过脱敏的 `message`。`RawSigV4.SelfTest` 单独记录 AWS 官方签名 golden、path-style URI、canonical query 与 Content-MD5 离线检查；Object Tagging 的 Put/Get、替换、TagCount、版本隔离、两种 CopyObject、删除、四类真实服务端负例和精确版本清理各自使用独立 `Tagging.*` operation；仅在异常路径执行的兜底清理记录为 `Tagging.ClientCleanup`。Object Lock 的 bucket 配置、对象状态、拒绝路径、bypass 与清理也分别记录；其异常兜底为 `ObjectLock.ClientCleanup`。报告不包含 Access Key secret、Authorization、数据库密码、主密钥、Silo root secret 或命令原始参数。

## 当前矩阵能力边界

当前矩阵将以下能力视为已实现并要求 `PASS`：

- Bucket create/head/list/delete。
- Put/Get/Head、单 Range 与条件读取。
- ListObjectsV2 prefix/delimiter/pagination。
- DeleteObjects。
- Versioning、`null` version、delete marker、精确 VersionId 读删。
- Multipart create/upload/list/complete/abort。
- CopyObject 与 UploadPartCopy。
- ListObjectVersions 与 ListMultipartUploads。
- Object Tagging，包括 PutObject 标签、精确版本 Get/Put/Delete、版本隔离、CopyObject COPY/REPLACE，以及 invalid key、duplicate key、超过 10 个标签、坏 percent-encoding 四项 Raw SigV4 服务端负例；TagCount 只对安装的 AWS CLI 实际暴露字段作断言。
- Bucket/Object Object Lock，包括默认 GOVERNANCE retention、显式对象 retention/legal hold、精确 VersionId 查询以及 governance bypass 的拒绝与允许路径。

仓库内还有以下不由 PowerShell native-client 矩阵执行、但属于当前兼容结论的自动化合同：

| 能力 | 当前实现 | 自动化证据 | 尚未覆盖 |
| --- | --- | --- | --- |
| Application quota | S3 Put/Copy/Multipart 的 reservation/commit/abort/expiry、Unversioned/Suspended null replacement、永久删除与 Lifecycle 均更新同一 `used_bytes/reserved_bytes` 账本；delete marker 不计容量 | Memory repository 测试与 PostgreSQL 17 `s3_quota_contract` 通过 | 独立真实 HTTP Copy 并发/故障注入 soak |
| Bucket Policy 管理 API | 严格 Core parser/evaluator、稳定 JSON、PolicyStatus；`0014` 提供稳定 12 位 account ID、全局 Bucket namespace 与 policy persistence；signed owner 支持 GET/PUT/DELETE `?policy` 和 GET `?policyStatus` | Core 单测、PostgreSQL 17 persistence contract，以及 PostgreSQL 17 SQLx HTTP 7/7 通过 | native-client Policy operation 尚未加入本 PowerShell 矩阵 |
| 数据面 Policy enforcement | GetObject、HeadObject、ListObjectsV2 已完成 | Identity/Bucket Deny 优先、同账户 union、跨账户 intersection、匿名 Allow、坏签名不降级与真实 PG17 HTTP contract | 其余写入、Copy、Multipart、版本列表、Tagging、Object Lock 和 Bucket 控制面尚未消费统一服务，当前这些 signed handler 仍暂时使用旧 `permissions` |

Tagging 的 invalid key、duplicate key、超过 10 个标签和坏 percent-encoding 是四个独立、必须 `PASS` 的负例 operation。前三项绕过 AWS CLI 本地模型校验，发送符合 [PutObjectTagging API](https://docs.aws.amazon.com/AmazonS3/latest/API/API_PutObjectTagging.html) 结构且 Content-MD5 正确的 XML；第四项直接保留畸形 percent-encoding 原文。四项都断言服务端真实返回 4xx 标准 S3 XML `InvalidTag`，不再用 CLI 本地拒绝或 `SKIP` 代替服务端协议结果。AWS CLI 缺失时，由于 bucket 和精确 seed version 无法建立，整组 AWS operation 仍按客户端缺失语义记为 `SKIP`；可单独运行 `Test-S3Compat.RawSigV4.ps1` 完成无网络离线校验。

Raw signer 的签名计算以 AWS 官方 [S3 SigV4 header authentication 示例](https://docs.aws.amazon.com/AmazonS3/latest/developerguide/sig-v4-header-based-auth.html) 为离线 golden，使用系统 .NET 的 SHA-256、HMAC-SHA256、MD5 与 `HttpWebRequest`，不加载 AWS SDK、第三方模块或外部 HTTP 命令。它按 UTF-8 字节生成 canonical URI，按编码后的 Name/Value 排序 canonical query，签入 host、Content-MD5、Content-Type、x-amz-content-sha256、x-amz-date 和测试附加头；请求固定为 path-style，禁止重定向。

Object Lock 矩阵只创建 GOVERNANCE retention，不创建测试时间内无法安全清理的 COMPLIANCE retention。PowerShell native-client 矩阵暂不覆盖 Bucket Policy/PolicyStatus、CORS、Notification、SSE、Lifecycle 执行效果和 virtual-host style；Bucket Policy/PolicyStatus 目前由上述 Core、PostgreSQL contract 与 SQLx HTTP 7/7 覆盖。anonymous GetObject/HeadObject/ListObjectsV2 与 signed read-policy enforcement 已由独立真实 PG17 HTTP contract 覆盖，但其余 action 仍是待完成项，不能以读取闭环替代。其他能力应在相应协议切片实现后以独立断言加入，不能以现有 `SKIP` 或其他操作的成功替代。

## 安全与清理

- 所有 native-client 输出在写终端和报告之前替换已注册 secret 及其 URL 编码形式。
- Raw signer 只接受绝对 `http`/`https` endpoint，拒绝 userinfo、query、fragment，保留 endpoint base path 与 path-style bucket/key；请求体上限 1 MiB，超时范围 1–60 秒且默认 15 秒，响应正文最多读取 32 KiB、响应头最多保留 16 KiB。Helper 不输出请求、Secret 或 Authorization，返回错误前还会再次脱敏 Secret、URL 编码 Secret 与 Authorization 形态。
- AWS 和 rclone 通过进程环境接收凭据；mcli 使用进程级 `MC_HOST_prismark`，并将配置目录指向系统临时目录。
- Docker `run` 使用 `--env NAME` 从当前进程传值，脚本日志不打印 secret 值。
- Docker 资源带有唯一名称与 `prismark.s3-compat.run=<run-id>` label，但清理只操作本轮解析出的精确容器名和 network 名。
- Endpoint 模式不会删除环境中的非本轮资源；AWS CLI 在创建普通 bucket 与 Object Lock bucket 前都要求 HeadBucket 明确返回 404/NoSuchBucket，无法证明名称未占用时直接 `FAIL`，不会进入创建或清理。每个客户端使用带 run ID 的唯一 bucket，并在自身矩阵末尾删除。
- Tagging 矩阵在每次 PutObject/CopyObject 返回 opaque VersionId 后立即登记 `Key + VersionId`。四个负例各使用唯一 key；前三项的 seed version 先登记，坏 percent-encoding 若被错误接受，也只有响应携带精确、非 `null` 的 `x-amz-version-id` 才登记。正常及异常清理都只逐条删除这份内存清单并用 HeadObject 精确确认 404；不会调用 ListObjectVersions 遍历普通 bucket，也不会按 prefix 猜测或删除非本轮版本。若服务端创建对象却未返回 VersionId，矩阵宁可保留 bucket 并报告 `FAIL`，不会扩大删除范围。
- Object Lock 正常路径和异常兜底都按“legal hold OFF → 使用已签名 bypass 删除仍处于 GOVERNANCE retention 的精确 VersionId → 删除 delete marker → 删除 bucket → HeadBucket 确认不存在”的顺序清理。不会等待 retention 到期，也不会创建 COMPLIANCE retention。
- 若出现 `FAIL` 导致外部 bucket 无法清空，报告会保留精确的 Object Lock cleanup 结果和该 bucket 的 run ID 以便人工检查；脚本不会改名匹配或遍历删除其他 bucket。
- 系统临时目录删除前会验证绝对路径位于系统 temp 下，且目录名匹配 `prismark-s3-compat-*`。

## 故障排查

1. `Client.Availability = SKIP`：确认命令在启动脚本的同一个 PowerShell `PATH` 中。
2. `Environment.Setup = FAIL`：检查 Docker Desktop、镜像拉取、Docker build 和 PrismArk 容器日志摘要。
3. `SignatureDoesNotMatch`：确认 endpoint 使用 Application AccessKey，而不是 PrismArk 底层 Silo/AWS 存储密钥；确认 region 为 `us-east-1` 或部署配置使用的 region。
4. Silo 模式启动失败：先固定一个已验证的 `-SiloImage` digest，确认镜像包含 `mcli`。
5. Endpoint 模式遗留 bucket：按报告中的 run ID 定位 `prismark-compat-<client>-<run-id>`，仅清理该 bucket。
6. `ObjectLock.ClientCleanup = FAIL`：按报告中的精确 bucket 名检查残留 VersionId；先将该版本 legal hold 设为 OFF，再使用有权限且正确签名的 `--bypass-governance-retention` 删除精确版本，最后删除 bucket。不要对同前缀的其他 bucket 做批量清理。
7. `Tagging.ClientCleanup = FAIL`：报告中的 bucket 仅属于该 run ID，但脚本不会遍历删除未知版本；根据此前失败 operation 返回的精确 key/versionId 做人工清理，不要对 `tagging/` 前缀执行批量删除。
