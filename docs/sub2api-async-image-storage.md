# sub2api 异步图片存储接入

MediaHub 提供一个面向 AWS SDK 的有限 S3 网关，使 `sub2api` 异步图片任务可以把结果写成正常的 MediaHub 对象。该入口与 MediaHub 自己使用的底层 Local/S3 Adapter 无关：无论 MediaHub 字节存储在本地磁盘、AWS S3、Cloudflare R2 或其他兼容后端，客户端配置都使用同一个 MediaHub `/s3` Endpoint。

## 支持范围

当前入站网关支持以下 AWS S3 操作：

| 功能 | S3 操作 | 支持边界 |
|---|---|---|
| 上传 | `PutObject` | Header-signed 请求；可通过 `x-amz-acl` 同时设置有限 canned ACL |
| 下载 | `GetObject`、`HeadObject` | Query-presigned 或 Header-signed 请求；支持单段 Range、ETag、Content-Length、Content-Type 和下载限速 |
| 列举 | `ListObjectsV2` | 支持 `prefix`、`delimiter=/`、`max-keys`、`start-after`、不透明 `continuation-token` 和 `encoding-type=url` |
| 删除 | `DeleteObject`、`DeleteObjects` | 删除不存在的 Key 也成功；批量请求最多 1000 个 Key，并进入 MediaHub 的延迟删除、Outbox 和审计流程 |
| 分片上传 | `CreateMultipartUpload`、`UploadPart`、`ListParts`、`CompleteMultipartUpload`、`AbortMultipartUpload` | 持久化 Upload/Part 状态；完成时校验 Part 顺序、ETag、独立 SHA-256、Bucket 策略和配额 |
| 对象 ACL | `GetObjectAcl`、`PutObjectAcl` | 仅支持 `private` 和 `public-read` canned ACL，并映射为 MediaHub 对象可见性 |
| 鉴权 | AWS SigV4 静态 AccessKey | Region 可由客户端选择，Service 必须为 `s3` |

它仍不是完整的 S3 实现。当前不支持 `ListObjects` V1、`CreateBucket`、Bucket ACL、Object Versioning 和 virtual-hosted-style Bucket。带 `versionId` 的对象请求会返回 `NotImplemented`。

ACL 是有意收窄的兼容层：`x-amz-acl` 只接受 `private` 或 `public-read`；`x-amz-grant-*` Header、ACL XML 请求体以及 `authenticated-read` 等其他 canned ACL 会返回明确的 S3 `AccessControlListNotSupported`，不会静默忽略授权设置。

## MediaHub 准备

1. 在目标 Application 下创建一个 Bucket，例如 `generated-images`。
2. 根据保留策略配置 Bucket 默认 TTL 或生命周期规则。通过 S3 网关写入的对象是正常 Media 记录，会进入 MediaHub 配额、审计、生命周期和控制台对象列表。
3. 为同一 Application 创建 AccessKey。仅供 `sub2api` 存取图片时至少授予 `media:upload` 和 `media:read`；要使用本页全部扩展操作，再授予 `media:list`、`media:update` 和 `media:delete`。
4. 立即保存创建响应中的 SecretAccessKey；MediaHub 后续不会再次展示明文。

建议为 `sub2api` 单独创建 AccessKey。当前 AccessKey 权限可以限制操作类型，但尚不能限制到单个 Bucket 或 Object Key Prefix。

扩展操作使用的权限如下：

| 权限 | 对应操作 |
|---|---|
| `media:upload` | `PutObject` 和全部 Multipart 操作 |
| `media:read` | `GetObject`、`HeadObject`、`GetObjectAcl` |
| `media:list` | `ListObjectsV2` |
| `media:update` | `PutObjectAcl`，以及在 `PutObject`/`CreateMultipartUpload` 中指定 `x-amz-acl` |
| `media:delete` | `DeleteObject`、`DeleteObjects` |

## sub2api 配置

在 `sub2api` 的 `/app/data/config.yaml` 中配置：

```yaml
image_storage:
  enabled: true
  endpoint: "https://media.example.com/s3"
  region: "us-east-1"
  bucket: "generated-images"
  access_key_id: "mh_ak_..."
  secret_access_key: "创建 AccessKey 时一次性返回的 SecretAccessKey"
  prefix: "images/"
  force_path_style: true
  public_base_url: ""
  presign_expiry_hours: 24
  max_download_bytes: 33554432
```

关键约束：

- `endpoint` 必须以 `/s3` 结尾，不能填写 MediaHub 根地址，也不能填写 MediaHub 的底层 R2/S3 Endpoint。
- `force_path_style` 必须为 `true`。Bucket 必须出现在 `/s3/{bucket}/{key}` 路径中。
- 私有 Bucket 的 `public_base_url` 必须留空，让 `sub2api` 返回 AWS query-presigned GET URL。
- `region` 推荐固定为 `us-east-1`；MediaHub 会验证请求 Credential Scope 前后一致，但不把 Region 映射到物理机房。
- S3 网关单请求上限为 64 MiB。建议 `max_download_bytes` 不超过 `67108864`；默认 32 MiB 可直接使用。
- MediaHub Object Key 不可覆盖。`sub2api` 使用任务 ID 生成 Key，正常情况下不会冲突；重放或重复 Key 会返回 S3 `OperationAborted`。

修改挂载的 `config.yaml` 后重启：

```bash
docker compose restart sub2api
```

如果使用 `IMAGE_STORAGE_*` 环境变量，必须把变量实际传入 `sub2api` 容器。修改 Compose `environment` 或 `env_file` 后重新创建容器：

```yaml
services:
  sub2api:
    environment:
      IMAGE_STORAGE_ENABLED: "true"
      IMAGE_STORAGE_ENDPOINT: "https://media.example.com/s3"
      IMAGE_STORAGE_REGION: "us-east-1"
      IMAGE_STORAGE_BUCKET: "generated-images"
      IMAGE_STORAGE_ACCESS_KEY_ID: "mh_ak_..."
      IMAGE_STORAGE_SECRET_ACCESS_KEY: "创建 AccessKey 时一次性返回的 SecretAccessKey"
      IMAGE_STORAGE_PREFIX: "images/"
      IMAGE_STORAGE_FORCE_PATH_STYLE: "true"
      IMAGE_STORAGE_PUBLIC_BASE_URL: ""
      IMAGE_STORAGE_PRESIGN_EXPIRY_HOURS: "24"
      IMAGE_STORAGE_MAX_DOWNLOAD_BYTES: "33554432"
```

```bash
docker compose up -d --force-recreate sub2api
```

这些配置只决定 `sub2api` 如何访问 MediaHub 入站网关。List、Delete、Multipart 和有限 ACL 继续使用同一个 `/s3` Endpoint 和 AccessKey，不需要额外配置 MediaHub 的底层 Local/S3 Adapter，也不要把 MediaHub 自己访问 R2/S3 的凭据交给 `sub2api`。

## Multipart 限制

- 每个 Application 最多同时保留 1000 个 `pending` 或 `completing` 状态的 Multipart Upload；超出后必须先完成、中止或等待旧 Upload 过期。
- 每个 `UploadPart` 请求体最多 64 MiB，Part Number 范围为 1 到 10000。除最后一个 Part 外，每个 Part 至少 5 MiB。
- `UploadPart` 写入 Part 元数据时即预占 Application 配额；覆盖同一 Part Number 时按新旧 Part 大小的差值增加或释放预留量。累计上传量同时受目标 Bucket 的 `max_object_size` 和 MediaHub 2 GiB 单对象技术上限约束。
- `CompleteMultipartUpload` 将该 Upload 的预留配额转为正式 Media 已用配额；`AbortMultipartUpload` 和过期回收会释放预留配额。Multipart Upload 默认 24 小时过期，完成或中止后会清理临时 Part，生命周期 Worker 也会回收过期 Upload。
- 完成后的总对象大小必须同时满足目标 Bucket 的 `max_object_size`、Application 配额和 MediaHub 2 GiB 单对象技术上限；三者取更小值。
- `CompleteMultipartUpload` 必须提交按 Part Number 升序排列的已上传 Part 和对应 ETag。重复或缺失 Part、ETag 不匹配会返回对应的 S3 XML 错误。
- MediaHub Object Key 仍不可覆盖。完成时若同一 Key 已被不同内容占用，会返回 `OperationAborted`。

## 反向代理要求

- 生产环境必须使用 HTTPS。
- 保留原始 `Host`、请求路径、查询字符串、`Authorization` 和 `X-Amz-*` Header。
- 不得对 `/s3` 后的路径做二次 URL decode、路径归一化或重写。
- 允许至少 64 MiB 请求体，并关闭会改写已签名 Header/Query 的规则。
- 预签名 URL 的 Host 必须与客户端签名时使用的外部 Host 相同。

## 验证

重启 `sub2api` 后提交一次异步图片任务：

```bash
curl -i https://sub2api.example.com/v1/images/generations/async \
  -H 'Authorization: Bearer sk-...' \
  -H 'Content-Type: application/json' \
  -d '{"model":"gpt-image-1","prompt":"A lighthouse during a winter storm"}'
```

验收条件：

1. 提交返回 `202 Accepted`，不再因对象存储未配置而返回 `404`。
2. 轮询任务最终为 `completed`，`result.data[].url` 存在且没有 `b64_json`。
3. URL 能返回正确图片；私有 Bucket 响应包含 `private, no-store`。
4. MediaHub 控制台目标 Bucket 中出现 `images/imgtask_...` 对象。
5. Audit Log 中上传事件的 Actor 是该 AccessKey，摘要包含 `protocol: s3`。

`GET /api/v1/capabilities` 的 `s3_gateway: true` 表示当前部署包含该入口。
