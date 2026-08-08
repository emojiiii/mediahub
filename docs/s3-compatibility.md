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

每个 result 包含 `client`、`operation`、`status`、`duration_ms` 和经过脱敏的 `message`。报告不包含 Access Key secret、数据库密码、主密钥、Silo root secret 或命令原始参数。

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

矩阵暂不覆盖 Bucket Policy、Tagging、CORS、Notification、SSE、Object Lock、Lifecycle 执行效果和 virtual-host style。Bucket/Object Object Lock 已在 Rust 的 SigV4 HTTP 与 PostgreSQL 合同测试中覆盖，后续会加入 native-client 矩阵。其他能力应在相应协议切片实现后以独立断言加入，不能以现有 `SKIP` 或其他操作的成功替代。

## 安全与清理

- 所有 native-client 输出在写终端和报告之前替换已注册 secret 及其 URL 编码形式。
- AWS 和 rclone 通过进程环境接收凭据；mcli 使用进程级 `MC_HOST_prismark`，并将配置目录指向系统临时目录。
- Docker `run` 使用 `--env NAME` 从当前进程传值，脚本日志不打印 secret 值。
- Docker 资源带有唯一名称与 `prismark.s3-compat.run=<run-id>` label，但清理只操作本轮解析出的精确容器名和 network 名。
- Endpoint 模式不会删除环境中的非本轮资源；每个客户端使用唯一 bucket，并在自身矩阵末尾删除。若出现 `FAIL` 导致外部 bucket 无法清空，报告会保留该 bucket 的 run ID 以便人工检查。
- 系统临时目录删除前会验证绝对路径位于系统 temp 下，且目录名匹配 `prismark-s3-compat-*`。

## 故障排查

1. `Client.Availability = SKIP`：确认命令在启动脚本的同一个 PowerShell `PATH` 中。
2. `Environment.Setup = FAIL`：检查 Docker Desktop、镜像拉取、Docker build 和 PrismArk 容器日志摘要。
3. `SignatureDoesNotMatch`：确认 endpoint 使用 Application AccessKey，而不是 PrismArk 底层 Silo/AWS 存储密钥；确认 region 为 `us-east-1` 或部署配置使用的 region。
4. Silo 模式启动失败：先固定一个已验证的 `-SiloImage` digest，确认镜像包含 `mcli`。
5. Endpoint 模式遗留 bucket：按报告中的 run ID 定位 `prismark-compat-<client>-<run-id>`，仅清理该 bucket。
