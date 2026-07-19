import { createMediaHubClient, resolveApiBaseUrl } from './client'
import type { components } from './generated'
import { sha256File } from '../upload-hash'

export const apiBaseUrl = resolveApiBaseUrl(import.meta.env.VITE_API_BASE_URL?.trim())

export type Visibility = 'public' | 'private'

export type Application = {
  id: string
  name: string
  appId: string
  usedBytes: number
  quotaBytes: number
  status: 'active' | 'suspended'
}

export type ObjectItem = {
  id: string
  name: string
  key: string
  bucket: string
  bucketId: string
  type: string
  size: number
  sha256: string
  revision: number
  createdAt: string
  status: string
  visibility: '公开' | '私有'
}

export type Bucket = {
  id: string
  name: string
  visibility: '公开' | '私有'
  objectCount: number
  used: string
  lifecycle: string
  defaultTtlSeconds: number | null
  maxObjectSize: number | null
  allowedMimeTypes: string[]
  lifecycleRules: LifecycleRule[]
}

export type LifecycleRule = {
  id: string
  enabled: boolean
  prefix: string
} & ({ type: 'expire_after'; durationSeconds: number } | { type: 'keep_latest'; count: number })

export type Permission = components['schemas']['Permission']
export type AccessKey = {
  id: string
  name: string
  permissions: Permission[]
  scope: string
  lastUsed: string
  expires: string
  expiresAt: string | null
  status: '有效' | '已撤销'
}

export type WebhookEndpoint = {
  id: string
  url: string
  events: string[]
  enabled: boolean
  state: '健康' | '已停用'
  latest: string
}
export type MediaStatus = components['schemas']['Media']['state']
export type MediaFilters = {
  bucket?: string
  status?: MediaStatus
  mime?: string
  createdFrom?: string
  createdBefore?: string
  prefix?: string
  delimiter?: '/'
  limit?: number
  cursor?: string
}
export type MediaPage = { items: ObjectItem[]; commonPrefixes: string[]; nextCursor: string | null }
export type WebhookDeliveryStatus = 'pending' | 'delivered' | 'dead_lettered'
export type WebhookDelivery = {
  eventId: string
  endpointId: string
  eventType: string
  attemptCount: number
  status: WebhookDeliveryStatus
  lastResponseStatus: number | null
  lastError: string | null
  createdAt: string
  updatedAt: string
  nextAttemptAt: string | null
  deliveredAt: string | null
  deadLetteredAt: string | null
  replayCount: number
  lastReplayedAt: string | null
}
export type WebhookDeliveryFilters = { status?: WebhookDeliveryStatus; limit?: number; cursor?: string }
export type WebhookDeliveryPage = { items: WebhookDelivery[]; nextCursor: string | null }

export type User = { name: string; email: string; systemRole: 'user' | 'admin' }
export type RegistrationResult = { email: string; status: 'pending_verification'; verificationToken?: string }
export type ResendVerificationResult = { message: string; verificationToken?: string }
export type ForgotPasswordResult = { message: string; resetToken?: string }
export type AuthSession = {
  id: string
  expiresAt: string
  lastSeenAt: string
  createdIp: string | null
  lastSeenIp: string | null
  userAgent: string | null
  createdAt: string
  isCurrent: boolean
}
export type Capabilities = { storageBackend: string; imageProcessing: boolean; videoProcessing: boolean; resumableUpload: boolean }
type DashboardBucket = { name: string; objects: number; used: string; share: number }
export type Dashboard = {
  app: Application
  objectCount: number
  todayUploads: number
  todayDeletes: number
  requests: number
  operationalMetricsAvailable: boolean
  storageBackend: string
  imageProcessing: boolean
  buckets: DashboardBucket[]
  mime: Array<{ label: string; amount: number; color: string }>
}
export type AdminUser = {
  id: string
  email: string
  status: 'pending_verification' | 'active' | 'suspended'
  systemRole: 'user' | 'admin'
  emailVerifiedAt: string | null
  lastLoginAt: string | null
  createdAt: string
  updatedAt: string
}
export type AdminApplication = { id: string; ownerUserId: string; appId: string; name: string; quotaBytes: number; usedBytes: number; reservedBytes: number; createdAt: string; updatedAt: string }
export type AdminJob = { id: string; applicationId: string; action: string; state: JobState; totalItems: number; succeededItems: number; failedItems: number; attemptCount: number; maxAttempts: number; errorSummary: string | null; createdAt: string; updatedAt: string }
export type AdminStorage = { quotaBytes: number; usedBytes: number; reservedBytes: number; mediaObjects: number; variantBytes: number; variants: number; diskTotalBytes: number; diskAvailableBytes: number }
export type AdminSystemSettings = { downloadBytesPerSecond: number | null; updatedAt: string }
export type AdminAudit = { id: string; applicationId: string; actorType: 'user' | 'access_key' | 'system'; actorId: string; action: string; targetType: string; targetId: string; requestId: string; summary: Record<string, unknown>; createdAt: string }

export type BucketInput = {
  name: string
  visibility: Visibility
  defaultTtlSeconds: number | null
  maxObjectSize: number | null
  allowedMimeTypes: string[]
  lifecycleRules: LifecycleRule[]
}
export type AccessKeyInput = { name: string; permissions: Permission[]; expiresAt: string | null }
export type WebhookInput = { url: string; events: string[]; enabled: boolean }
export type OneTimeSecret = { title: string; identifier: string; secret: string }
export type MediaUpdateInput = {
  displayName?: string
  visibility?: Visibility | null
  ttlSeconds?: number | null
  metadata?: { user?: Record<string, unknown>; ai?: Record<string, unknown> }
}
export type UploadProgress = 'creating' | 'uploading' | 'verifying'
export type UploadOptions = {
  bucket: string
  objectKey?: string
  signal?: AbortSignal
  onProgress?: (progress: UploadProgress) => void
  onSession?: (uploadId: string) => void
}
export type UploadSessionView = {
  uploadId: string
  mediaId: string
  bucketId: string
  objectKey: string
  expectedSize: number
  expectedMime: string
  state: 'pending' | 'completed' | 'cancelled' | 'expired'
  expiresAt: string
  updatedAt: string
}
export type ResumeUploadOptions = Pick<UploadOptions, 'signal' | 'onProgress'>
export type VariantParams = {
  width: number
  height: number
  fit: 'cover' | 'contain' | 'inside'
  quality: number
  format: 'jpeg' | 'png' | 'webp'
  blur: number
  crop: 'center' | 'top' | 'bottom' | 'left' | 'right'
  background: string
}
export type BatchAction =
  | { type: 'update_ttl_seconds'; ttl_seconds: number | null }
  | { type: 'update_visibility'; visibility: Visibility }
  | { type: 'delete' }
export type BatchItemResult = {
  mediaId: string
  state: 'pending' | 'succeeded' | 'failed' | 'cancelled'
  errorCode?: string
  errorSummary?: string
}
export type JobState = 'pending' | 'running' | 'completed' | 'failed' | 'cancelled'
export type AsyncJobView = {
  id: string
  state: JobState
  action: BatchAction
  totalItems: number
  succeededItems: number
  failedItems: number
  errorSummary?: string
  createdAt: string
  updatedAt: string
  items: BatchItemResult[]
}
export type BatchOperationResult =
  | { mode: 'sync'; items: BatchItemResult[] }
  | { mode: 'job'; job: AsyncJobView }

export class ApiRequestError extends Error {
  constructor(readonly status: number, readonly code: string | undefined, message: string) {
    super(message)
    this.name = 'ApiRequestError'
  }
}

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : '请求失败，请稍后重试'
}

type Api = {
  setApplication(appId: string | undefined): void
  getMe(): Promise<User | null>
  register(email: string, password: string): Promise<RegistrationResult>
  verifyEmail(token: string): Promise<void>
  resendVerification(email: string): Promise<ResendVerificationResult>
  signIn(email: string, password: string): Promise<User>
  signOut(): Promise<void>
  forgotPassword(email: string): Promise<ForgotPasswordResult>
  resetPassword(token: string, password: string): Promise<void>
  getSessions(): Promise<AuthSession[]>
  revokeSession(sessionId: string): Promise<void>
  revokeAllSessions(): Promise<void>
  getCapabilities(): Promise<Capabilities>
  getApplications(): Promise<Application[]>
  createApplication(name: string): Promise<Application>
  updateApplication(appId: string, name: string): Promise<Application>
  deleteApplication(appId: string): Promise<void>
  getDashboard(appId: string): Promise<Dashboard>
  getAdminUsers(): Promise<AdminUser[]>
  updateAdminUserStatus(userId: string, status: 'active' | 'suspended'): Promise<AdminUser>
  getAdminApplications(): Promise<AdminApplication[]>
  updateAdminApplicationQuota(applicationId: string, quotaBytes: number): Promise<AdminApplication>
  getAdminJobs(): Promise<AdminJob[]>
  getAdminStorage(): Promise<AdminStorage>
  getAdminSystemSettings(): Promise<AdminSystemSettings>
  updateAdminSystemSettings(downloadBytesPerSecond: number | null): Promise<AdminSystemSettings>
  getAdminAudit(): Promise<AdminAudit[]>
  getObjects(filters?: MediaFilters): Promise<MediaPage>
  getObject(mediaId: string): Promise<ObjectItem>
  updateObject(mediaId: string, revision: number, input: MediaUpdateInput): Promise<void>
  deleteObject(mediaId: string): Promise<void>
  getSignedUrl(mediaId: string): Promise<{ url: string; expiresAt: string }>
  getVariantUrl(mediaId: string, params: VariantParams): Promise<{ url: string; expiresAt: string }>
  executeBatch(mediaIds: string[], action: BatchAction): Promise<BatchOperationResult>
  getJob(jobId: string): Promise<AsyncJobView>
  cancelJob(jobId: string): Promise<void>
  uploadFile(file: File, options: UploadOptions): Promise<ObjectItem>
  getUploadSession(uploadId: string): Promise<UploadSessionView>
  resumeUpload(uploadId: string, file: File, options?: ResumeUploadOptions): Promise<ObjectItem>
  cancelUpload(uploadId: string): Promise<void>
  getBuckets(): Promise<Bucket[]>
  createBucket(input: BucketInput): Promise<Bucket>
  updateBucket(name: string, input: Omit<BucketInput, 'name'>): Promise<Bucket>
  deleteBucket(name: string): Promise<void>
  getAccessKeys(appId: string): Promise<AccessKey[]>
  createAccessKey(appId: string, input: AccessKeyInput): Promise<OneTimeSecret>
  updateAccessKey(accessKeyId: string, input: AccessKeyInput): Promise<AccessKey>
  rotateAccessKey(appId: string, key: AccessKey): Promise<OneTimeSecret>
  revokeAccessKey(accessKeyId: string): Promise<void>
  getWebhooks(): Promise<WebhookEndpoint[]>
  getWebhookDeliveries(webhookId: string, filters?: WebhookDeliveryFilters): Promise<WebhookDeliveryPage>
  replayWebhookDelivery(webhookId: string, eventId: string): Promise<void>
  createWebhook(input: WebhookInput): Promise<OneTimeSecret>
  updateWebhook(webhookId: string, input: WebhookInput): Promise<WebhookEndpoint>
  rotateWebhookSecret(webhookId: string): Promise<OneTimeSecret>
  deleteWebhook(webhookId: string): Promise<void>
}


function lifecycleSummary(defaultTtlSeconds: number | null, rules: LifecycleRule[]): string {
  const enabled = rules.filter((rule) => rule.enabled).length
  if (enabled > 0) return `${enabled} 条启用规则${defaultTtlSeconds ? `，默认 TTL ${defaultTtlSeconds} 秒` : ''}`
  return defaultTtlSeconds ? `${defaultTtlSeconds} 秒后删除` : '默认永久保存'
}

type BackendMe = components['schemas']['Me']
type BackendRegistration = components['schemas']['RegistrationResponse']
type BackendResendVerification = components['schemas']['ResendVerificationResponse']
type BackendForgotPassword = components['schemas']['ForgotPasswordResponse']
type BackendSession = components['schemas']['Session']
type BackendCapabilities = components['schemas']['Capabilities']
type BackendApplication = components['schemas']['Application']
type BackendLifecycleRule = components['schemas']['LifecycleRule']
type BackendBucket = components['schemas']['Bucket']
type BackendMedia = components['schemas']['Media']
type BackendMediaPage = components['schemas']['MediaPage']
type BackendUpdateMedia = components['schemas']['UpdateMedia']
type BackendAccessKey = components['schemas']['AccessKey']
type BackendWebhook = components['schemas']['Webhook']
type BackendWebhookDelivery = components['schemas']['WebhookDelivery']
type BackendWebhookDeliveryPage = components['schemas']['WebhookDeliveryPage']
type BackendUploadSession = components['schemas']['CreateUploadSessionResponse']
type BackendUploadSessionView = components['schemas']['UploadSession']
type BackendAdminUser = components['schemas']['AdminUser']
type BackendAdminApplication = components['schemas']['AdminApplication']
type BackendAdminJob = components['schemas']['AdminJob']
type BackendAdminStorage = components['schemas']['AdminStorage']
type BackendAdminSystemSettings = components['schemas']['AdminSettings']
type BackendAdminAudit = components['schemas']['AdminAudit']

let selectedApplicationId: string | undefined
function csrfToken(): string | undefined {
  const prefix = 'mediahub_csrf='
  const cookie = document.cookie.split(';').map((part) => part.trim()).find((part) => part.startsWith(prefix))
  if (!cookie) return undefined
  try { return decodeURIComponent(cookie.slice(prefix.length)) } catch { return undefined }
}
const backendClient = createMediaHubClient(apiBaseUrl, csrfToken, () => selectedApplicationId)
type ClientResult<T> = { data?: T; error?: unknown; response: Response }

function clientError(response: Response, error: unknown): ApiRequestError {
  const envelope = valueRecord(error)
  const detail = valueRecord(envelope.error)
  return new ApiRequestError(response.status, typeof detail.code === 'string' ? detail.code : undefined, typeof detail.message === 'string' ? detail.message : `请求失败（HTTP ${response.status}）`)
}

async function backendResult<T>(request: PromiseLike<ClientResult<T>>): Promise<{ data: T; response: Response }> {
  const result = await request
  if (result.error !== undefined || !result.response.ok) throw clientError(result.response, result.error)
  if (result.data === undefined) throw new ApiRequestError(result.response.status, 'invalid_response', '服务端响应缺少数据')
  return { data: result.data, response: result.response }
}

async function backendData<T>(request: PromiseLike<ClientResult<T>>): Promise<T> { return (await backendResult(request)).data }
async function backendOk(request: PromiseLike<ClientResult<unknown>>): Promise<void> { const result = await request; if (result.error !== undefined || !result.response.ok) throw clientError(result.response, result.error) }
const bytes = (value: number) => Number.isFinite(value) && value >= 0 ? value : 0
function formatStorage(value: number): string { if (value < 1024 ** 2) return `${Math.max(0, Math.round(value / 1024))} KB`; if (value < 1024 ** 3) return `${(value / 1024 ** 2).toFixed(1)} MB`; return `${(value / 1024 ** 3).toFixed(2)} GB` }
const visibilityLabel = (visibility: Visibility): ObjectItem['visibility'] => visibility === 'public' ? '公开' : '私有'
const userFromMe = (me: BackendMe): User => ({ name: me.email.split('@')[0] || me.email, email: me.email, systemRole: me.system_role })
const applicationFromBackend = (app: BackendApplication): Application => ({ id: app.id, name: app.name, appId: app.app_id, usedBytes: bytes(app.used_bytes), quotaBytes: bytes(app.quota_bytes), status: 'active' })
const applicationFromMe = (me: BackendMe): Application => applicationFromBackend({ id: me.application_id, name: '默认应用', app_id: me.app_id, quota_bytes: me.quota_bytes, used_bytes: me.used_bytes, reserved_bytes: me.reserved_bytes })
function accessKeyFromBackend(key: BackendAccessKey): AccessKey { return { id: key.access_key_id, name: key.name, permissions: key.permissions, scope: key.permissions.join('，') || '无权限', lastUsed: '暂无使用记录', expires: key.expires_at ?? '不过期', expiresAt: key.expires_at ?? null, status: key.revoked_at ? '已撤销' : '有效' } }
function webhookFromBackend(endpoint: BackendWebhook): WebhookEndpoint { return { id: endpoint.id, url: endpoint.url, events: endpoint.events, enabled: endpoint.enabled, state: endpoint.enabled ? '健康' : '已停用', latest: '暂无投递记录' } }
function mediaName(media: BackendMedia): string { const segments = media.object_key.split('/').filter(Boolean); return media.display_name || segments[segments.length - 1] || media.id }
function lifecycleRuleFromBackend(rule: BackendLifecycleRule): LifecycleRule { return rule.type === 'expire_after' ? { id: rule.id, enabled: rule.enabled, prefix: rule.prefix, type: rule.type, durationSeconds: rule.duration_seconds } : { id: rule.id, enabled: rule.enabled, prefix: rule.prefix, type: rule.type, count: rule.count } }
function lifecycleRuleToBackend(rule: LifecycleRule): BackendLifecycleRule { return rule.type === 'expire_after' ? { id: rule.id, enabled: rule.enabled, prefix: rule.prefix, type: rule.type, duration_seconds: rule.durationSeconds } : { id: rule.id, enabled: rule.enabled, prefix: rule.prefix, type: rule.type, count: rule.count } }
function objectFromMedia(media: BackendMedia, bucketById: Map<string, BackendBucket>): ObjectItem { const bucket = bucketById.get(media.bucket_id); return { id: media.id, name: mediaName(media), key: media.object_key, bucket: bucket?.name ?? media.bucket_id, bucketId: media.bucket_id, type: media.mime, size: bytes(media.size_bytes), sha256: media.sha256, revision: media.revision, createdAt: media.created_at, status: media.state, visibility: media.visibility ? visibilityLabel(media.visibility) : bucket ? visibilityLabel(bucket.visibility) : '私有' } }
function bucketFromBackend(bucket: BackendBucket, media: BackendMedia[]): Bucket { const items = media.filter((item) => item.bucket_id === bucket.id); const used = items.reduce((sum, item) => sum + bytes(item.size_bytes), 0); const lifecycleRules = bucket.lifecycle_rules.map(lifecycleRuleFromBackend); const defaultTtlSeconds = bucket.default_ttl_seconds ?? null; return { id: bucket.id, name: bucket.name, visibility: visibilityLabel(bucket.visibility), objectCount: items.length, used: formatStorage(used), lifecycle: lifecycleSummary(defaultTtlSeconds, lifecycleRules), defaultTtlSeconds, maxObjectSize: bucket.max_object_size ?? null, allowedMimeTypes: bucket.allowed_mime_types, lifecycleRules } }
function metadataForBackend(metadata: MediaUpdateInput['metadata']): BackendUpdateMedia['metadata'] { return metadata as BackendUpdateMedia['metadata'] }
function bucketStats(buckets: BackendBucket[], media: BackendMedia[]): DashboardBucket[] { const total = media.reduce((sum, item) => sum + bytes(item.size_bytes), 0); return buckets.map((bucket) => { const items = media.filter((item) => item.bucket_id === bucket.id); const used = items.reduce((sum, item) => sum + bytes(item.size_bytes), 0); return { name: bucket.name, objects: items.length, used: formatStorage(used), share: total ? Math.round(used / total * 100) : 0 } }) }
function mimeBreakdown(media: Array<{ mime: string; size_bytes: number }>) {
  const total = media.reduce((sum, item) => sum + bytes(item.size_bytes), 0)
  const image = media.filter((item) => item.mime.startsWith('image/')).reduce((sum, item) => sum + bytes(item.size_bytes), 0)
  const video = media.filter((item) => item.mime.startsWith('video/')).reduce((sum, item) => sum + bytes(item.size_bytes), 0)
  const percentage = (size: number) => total ? Math.round(size / total * 100) : 0
  return [
    { label: '图像', amount: percentage(image), color: '#b9e64b' },
    { label: '视频', amount: percentage(video), color: '#ee8d61' },
    { label: '其他', amount: percentage(total - image - video), color: '#d5d5ce' },
  ]
}
function uploadSessionFromBackend(session: BackendUploadSessionView): UploadSessionView { return { uploadId: session.upload_id, mediaId: session.media_id, bucketId: session.bucket_id, objectKey: session.object_key, expectedSize: bytes(session.expected_size), expectedMime: session.expected_mime, state: session.state, expiresAt: session.expires_at, updatedAt: session.updated_at } }
function adminUserFromBackend(user: BackendAdminUser): AdminUser { return { id: user.id, email: user.email, status: user.status, systemRole: user.system_role, emailVerifiedAt: user.email_verified_at ?? null, lastLoginAt: user.last_login_at ?? null, createdAt: user.created_at, updatedAt: user.updated_at } }
function adminApplicationFromBackend(application: BackendAdminApplication): AdminApplication { return { id: application.id, ownerUserId: application.owner_user_id, appId: application.app_id, name: application.name, quotaBytes: bytes(application.quota_bytes), usedBytes: bytes(application.used_bytes), reservedBytes: bytes(application.reserved_bytes), createdAt: application.created_at, updatedAt: application.updated_at } }
function adminJobFromBackend(job: BackendAdminJob): AdminJob { return { id: job.id, applicationId: job.application_id, action: job.action, state: job.state, totalItems: job.total_items, succeededItems: job.succeeded_items, failedItems: job.failed_items, attemptCount: job.attempt_count, maxAttempts: job.max_attempts, errorSummary: job.error_summary ?? null, createdAt: job.created_at, updatedAt: job.updated_at } }
function adminStorageFromBackend(storage: BackendAdminStorage): AdminStorage { return { quotaBytes: bytes(storage.quota_bytes), usedBytes: bytes(storage.used_bytes), reservedBytes: bytes(storage.reserved_bytes), mediaObjects: storage.media_objects, variantBytes: bytes(storage.variant_bytes), variants: storage.variants, diskTotalBytes: bytes(storage.disk_total_bytes), diskAvailableBytes: bytes(storage.disk_available_bytes) } }
function adminSystemSettingsFromBackend(settings: BackendAdminSystemSettings): AdminSystemSettings { return { downloadBytesPerSecond: settings.download_bytes_per_second === null ? null : bytes(settings.download_bytes_per_second), updatedAt: settings.updated_at } }
function adminAuditFromBackend(audit: BackendAdminAudit): AdminAudit { return { id: audit.id, applicationId: audit.application_id, actorType: audit.actor_type, actorId: audit.actor_id, action: audit.action, targetType: audit.target_type, targetId: audit.target_id, requestId: audit.request_id, summary: audit.summary, createdAt: audit.created_at } }
async function requiredMe(): Promise<BackendMe> { return backendData(backendClient.GET('/api/v1/auth/me')) }
async function backendAllMedia(): Promise<BackendMedia[]> {
  const items: BackendMedia[] = []
  let cursor: string | undefined
  const seenCursors = new Set<string>()
  for (let pageNumber = 0; pageNumber < 10_000; pageNumber += 1) {
    const page = await backendData(backendClient.GET('/api/v1/media', { params: { query: { limit: 100, cursor } } }))
    items.push(...page.items)
    if (!page.next_cursor) break
    if (seenCursors.has(page.next_cursor)) throw new Error('媒体列表返回了重复 Cursor')
    seenCursors.add(page.next_cursor)
    cursor = page.next_cursor
    if (pageNumber === 9_999) throw new Error('媒体列表超过全量统计的安全上限')
  }
  return items
}
async function backendBucketsAndMedia(): Promise<[BackendBucket[], BackendMedia[]]> { return Promise.all([backendData(backendClient.GET('/api/v1/buckets')), backendAllMedia()]) }
async function backendObjectById(mediaId: string): Promise<{ appId: string; bucket: BackendBucket; media: BackendMedia; buckets: BackendBucket[] }> {
  const [appId, [buckets, media]] = await Promise.all([
    selectedApplicationId ? Promise.resolve(selectedApplicationId) : requiredMe().then((me) => me.app_id),
    backendBucketsAndMedia(),
  ])
  const item = media.find((value) => value.id === mediaId)
  if (!item) throw new ApiRequestError(404, 'not_found', 'Object not found')
  const bucket = buckets.find((value) => value.id === item.bucket_id)
  if (!bucket) throw new ApiRequestError(404, 'not_found', 'Object Bucket not found')
  return { appId, bucket, media: item, buckets }
}
function absoluteResourceUrl(path: string): string { return new URL(path, `${apiBaseUrl}/`).toString() }
function variantUrl(baseUrl: string, params: VariantParams): string {
  const url = new URL(baseUrl)
  url.searchParams.set('w', String(params.width))
  url.searchParams.set('h', String(params.height))
  url.searchParams.set('fit', params.fit)
  url.searchParams.set('quality', String(params.quality))
  url.searchParams.set('format', params.format)
  url.searchParams.set('blur', String(params.blur))
  url.searchParams.set('crop', params.crop)
  url.searchParams.set('background', params.background.replace(/^#/, ''))
  return url.toString()
}

function valueRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {}
}

function batchItemFromBackend(value: unknown): BatchItemResult {
  const item = valueRecord(value)
  const nestedError = valueRecord(item.error)
  const rawState = String(item.state ?? item.status ?? (item.success === false ? 'failed' : 'succeeded'))
  const state = ['pending', 'succeeded', 'failed', 'cancelled'].includes(rawState) ? rawState as BatchItemResult['state'] : rawState === 'ok' ? 'succeeded' : 'failed'
  return { mediaId: String(item.media_id ?? item.mediaId ?? item.id ?? ''), state, errorCode: item.error_code ? String(item.error_code) : nestedError.code ? String(nestedError.code) : undefined, errorSummary: item.error_summary ? String(item.error_summary) : nestedError.message ? String(nestedError.message) : typeof item.error === 'string' ? item.error : undefined }
}

function actionFromBackend(value: unknown): BatchAction {
  const action = valueRecord(value)
  if (action.type === 'update_ttl_seconds') return { type: 'update_ttl_seconds', ttl_seconds: typeof action.ttl_seconds === 'number' ? action.ttl_seconds : null }
  if (action.type === 'update_visibility') return { type: 'update_visibility', visibility: action.visibility === 'public' ? 'public' : 'private' }
  return { type: 'delete' }
}

function jobFromBackend(payload: unknown): AsyncJobView {
  const outer = valueRecord(payload)
  const job = valueRecord(outer.job ?? payload)
  const id = String(job.id ?? job.job_id ?? outer.job_id ?? '')
  if (!id) throw new Error('Job 响应缺少 ID')
  const stateValue = String(job.state ?? 'pending')
  const state = ['pending', 'running', 'completed', 'failed', 'cancelled'].includes(stateValue) ? stateValue as JobState : 'failed'
  const rawItems = outer.item_results ?? outer.items ?? job.item_results ?? job.items
  return {
    id,
    state,
    action: actionFromBackend(job.action),
    totalItems: Number(job.total_items ?? 0),
    succeededItems: Number(job.succeeded_items ?? 0),
    failedItems: Number(job.failed_items ?? 0),
    errorSummary: job.error_summary ? String(job.error_summary) : undefined,
    createdAt: String(job.created_at ?? new Date().toISOString()),
    updatedAt: String(job.updated_at ?? job.created_at ?? new Date().toISOString()),
    items: Array.isArray(rawItems) ? rawItems.map(batchItemFromBackend) : [],
  }
}

const backendApi: Api = {
  setApplication(appId) { selectedApplicationId = appId },
  async getMe() { try { return userFromMe(await requiredMe()) } catch (error) { if (error instanceof ApiRequestError && error.status === 401) return null; throw error } },
  async register(email, password) { const value: BackendRegistration = await backendData(backendClient.POST('/api/v1/auth/register', { body: { email, password } })); return { email: value.email, status: value.status, verificationToken: value.verification_token } },
  async verifyEmail(token) { await backendData(backendClient.POST('/api/v1/auth/verify-email', { body: { token } })) },
  async resendVerification(email) { const value: BackendResendVerification = await backendData(backendClient.POST('/api/v1/auth/resend-verification', { body: { email } })); return { message: '如果账号仍在等待验证，验证说明已经发出', verificationToken: value.verification_token } },
  async signIn(email, password) { return userFromMe(await backendData(backendClient.POST('/api/v1/auth/login', { body: { email, password } }))) },
  async signOut() { await backendOk(backendClient.POST('/api/v1/auth/logout')) },
  async forgotPassword(email) { const value: BackendForgotPassword = await backendData(backendClient.POST('/api/v1/auth/forgot-password', { body: { email } })); return { message: value.message, resetToken: value.reset_token } },
  async resetPassword(token, password) { await backendOk(backendClient.POST('/api/v1/auth/reset-password', { body: { token, password } })) },
  async getSessions() { return (await backendData(backendClient.GET('/api/v1/auth/sessions'))).map((session: BackendSession) => ({ id: session.id, expiresAt: session.expires_at, lastSeenAt: session.last_seen_at, createdIp: session.created_ip ?? null, lastSeenIp: session.last_seen_ip ?? null, userAgent: session.user_agent_summary ?? null, createdAt: session.created_at, isCurrent: session.is_current })) },
  async revokeSession(sessionId) { await backendOk(backendClient.DELETE('/api/v1/auth/sessions/{session_id}', { params: { path: { session_id: sessionId } } })) },
  async revokeAllSessions() { await backendOk(backendClient.DELETE('/api/v1/auth/sessions')) },
  async getCapabilities() { const value: BackendCapabilities = await backendData(backendClient.GET('/api/v1/capabilities')); return { storageBackend: value.storage[0] ?? 'unknown', imageProcessing: value.image_processing, videoProcessing: value.video_processing, resumableUpload: value.resumable_upload } },
  async getApplications() { return (await backendData(backendClient.GET('/api/v1/applications'))).map(applicationFromBackend) },
  async createApplication(name) { return applicationFromBackend(await backendData(backendClient.POST('/api/v1/applications', { body: { name } }))) },
  async updateApplication(appId, name) { return applicationFromBackend(await backendData(backendClient.PATCH('/api/v1/applications/{app_id}', { params: { path: { app_id: appId } }, body: { name } }))) },
  async deleteApplication(appId) { await backendOk(backendClient.DELETE('/api/v1/applications/{app_id}', { params: { path: { app_id: appId } } })) },
  async getDashboard(appId) { const [backendApps, [buckets, media], capabilities] = await Promise.all([backendData(backendClient.GET('/api/v1/applications')), backendBucketsAndMedia(), backendData(backendClient.GET('/api/v1/capabilities'))]); const app = backendApps.map(applicationFromBackend).find((item) => item.appId === appId) ?? applicationFromMe(await requiredMe()); return { app, objectCount: media.length, todayUploads: 0, todayDeletes: 0, requests: 0, operationalMetricsAvailable: false, storageBackend: capabilities.storage[0] ?? 'unknown', imageProcessing: capabilities.image_processing, buckets: bucketStats(buckets, media), mime: mimeBreakdown(media) } },
  async getAdminUsers() { return (await backendData(backendClient.GET('/api/v1/admin/users', { params: { query: { limit: 100 } } }))).map(adminUserFromBackend) },
  async updateAdminUserStatus(userId, status) { return adminUserFromBackend(await backendData(backendClient.PATCH('/api/v1/admin/users/{user_id}/status', { params: { path: { user_id: userId } }, body: { status } }))) },
  async getAdminApplications() { return (await backendData(backendClient.GET('/api/v1/admin/applications', { params: { query: { limit: 100 } } }))).map(adminApplicationFromBackend) },
  async updateAdminApplicationQuota(applicationId, quotaBytes) { return adminApplicationFromBackend(await backendData(backendClient.PATCH('/api/v1/admin/applications/{application_id}/quota', { params: { path: { application_id: applicationId } }, body: { quota_bytes: quotaBytes } }))) },
  async getAdminJobs() { return (await backendData(backendClient.GET('/api/v1/admin/jobs', { params: { query: { limit: 100 } } }))).map(adminJobFromBackend) },
  async getAdminStorage() { return adminStorageFromBackend(await backendData(backendClient.GET('/api/v1/admin/storage'))) },
  async getAdminSystemSettings() { return adminSystemSettingsFromBackend(await backendData(backendClient.GET('/api/v1/admin/settings'))) },
  async updateAdminSystemSettings(downloadBytesPerSecond) { return adminSystemSettingsFromBackend(await backendData(backendClient.PATCH('/api/v1/admin/settings', { body: { download_bytes_per_second: downloadBytesPerSecond } }))) },
  async getAdminAudit() { return (await backendData(backendClient.GET('/api/v1/admin/audit', { params: { query: { limit: 100 } } }))).map(adminAuditFromBackend) },
  async getObjects(filters = {}) { const [buckets, page] = await Promise.all([backendData(backendClient.GET('/api/v1/buckets')), backendData(backendClient.GET('/api/v1/media', { params: { query: { bucket: filters.bucket, status: filters.status, mime: filters.mime, created_from: filters.createdFrom, created_before: filters.createdBefore, prefix: filters.prefix, delimiter: filters.delimiter, limit: filters.limit, cursor: filters.cursor } } }))]); const map = new Map(buckets.map((bucket) => [bucket.id, bucket])); return { items: page.items.map((item) => objectFromMedia(item, map)), commonPrefixes: page.common_prefixes, nextCursor: page.next_cursor } },
  async getObject(mediaId) { const { buckets, media } = await backendObjectById(mediaId); return objectFromMedia(media, new Map(buckets.map((bucket) => [bucket.id, bucket]))) },
  async updateObject(mediaId, revision, input) { const { appId, bucket, media } = await backendObjectById(mediaId); await backendData(backendClient.PATCH('/{app_id}/{bucket}/{object_key}', { params: { path: { app_id: appId, bucket: bucket.name, object_key: media.object_key }, header: { 'If-Match': `"${revision}"` } }, body: { display_name: input.displayName, visibility: input.visibility, ttl_seconds: input.ttlSeconds, metadata: metadataForBackend(input.metadata) } })) },
  async deleteObject(mediaId) { const { appId, bucket, media } = await backendObjectById(mediaId); await backendOk(backendClient.DELETE('/{app_id}/{bucket}/{object_key}', { params: { path: { app_id: appId, bucket: bucket.name, object_key: media.object_key } } })) },
  async getSignedUrl(mediaId) { const { appId, bucket, media } = await backendObjectById(mediaId); const value = await backendData(backendClient.POST('/{app_id}/{bucket}/{object_key}', { params: { path: { app_id: appId, bucket: bucket.name, object_key: media.object_key } } })); return { url: absoluteResourceUrl(value.url), expiresAt: value.expires_at } },
  async getVariantUrl(mediaId, params) { const signed = await backendApi.getSignedUrl(mediaId); return { ...signed, url: variantUrl(signed.url, params) } },
  async executeBatch(mediaIds, action) {
    const payload = await backendData(backendClient.POST('/api/v1/media/batch', { params: { header: { 'Idempotency-Key': crypto.randomUUID() } }, body: { action, media_ids: mediaIds } }))
    if ('job' in payload) return { mode: 'job', job: jobFromBackend(payload) }
    return { mode: 'sync', items: payload.results.map(batchItemFromBackend) }
  },
  async getJob(jobId) { return jobFromBackend(await backendData(backendClient.GET('/api/v1/jobs/{job_id}', { params: { path: { job_id: jobId } } }))) },
  async cancelJob(jobId) { await backendData(backendClient.DELETE('/api/v1/jobs/{job_id}', { params: { path: { job_id: jobId } } })) },
  async uploadFile(file, options) {
    options.onProgress?.('creating')
    const session: BackendUploadSession = await backendData(backendClient.POST('/api/v1/uploads', { body: { bucket: options.bucket, object_key: options.objectKey, original_name: file.name, display_name: file.name, expected_size: file.size, content_type: file.type || 'application/octet-stream' } }))
    options.onSession?.(session.upload_id)
    try {
      options.onProgress?.('uploading')
      const headers = new Headers(session.headers); headers.set('Content-Type', session.expected_mime)
      const put = await fetch(absoluteResourceUrl(session.url), { method: session.method, headers, body: file, signal: options.signal })
      if (!put.ok) throw clientError(put, await put.json().catch(() => undefined))
      options.onProgress?.('verifying')
      const digest = await sha256File(file, options.signal)
      const complete = await backendData(backendClient.POST('/api/v1/uploads/{upload_session_id}/complete', { params: { path: { upload_session_id: session.upload_id } }, body: { sha256: digest }, signal: options.signal }))
      const buckets = await backendData(backendClient.GET('/api/v1/buckets'))
      return objectFromMedia(complete.media, new Map(buckets.map((bucket) => [bucket.id, bucket])))
    } catch (error) { throw error }
  },
  async getUploadSession(uploadId) { return uploadSessionFromBackend(await backendData(backendClient.GET('/api/v1/uploads/{upload_session_id}', { params: { path: { upload_session_id: uploadId } } }))) },
  async resumeUpload(uploadId, file, options = {}) {
    const session = await backendData(backendClient.GET('/api/v1/uploads/{upload_session_id}', { params: { path: { upload_session_id: uploadId } } }))
    if (session.state !== 'pending' || !session.upload_target) throw new ApiRequestError(409, 'conflict', 'UploadSession 已结束或无法继续')
    if (file.size !== session.expected_size || (file.type || 'application/octet-stream') !== session.expected_mime) throw new ApiRequestError(422, 'upload_mismatch', '文件大小或 MIME 类型与 UploadSession 不匹配')
    options.onProgress?.('uploading')
    const headers = new Headers(session.upload_target.headers); headers.set('Content-Type', session.expected_mime)
    const put = await fetch(absoluteResourceUrl(session.upload_target.url), { method: session.upload_target.method, headers, body: file, signal: options.signal })
    if (!put.ok) throw clientError(put, await put.json().catch(() => undefined))
    options.onProgress?.('verifying')
    const complete = await backendData(backendClient.POST('/api/v1/uploads/{upload_session_id}/complete', { params: { path: { upload_session_id: uploadId } }, body: { sha256: await sha256File(file, options.signal) }, signal: options.signal }))
    const buckets = await backendData(backendClient.GET('/api/v1/buckets'))
    return objectFromMedia(complete.media, new Map(buckets.map((bucket) => [bucket.id, bucket])))
  },
  async cancelUpload(uploadId) { await backendOk(backendClient.DELETE('/api/v1/uploads/{upload_session_id}', { params: { path: { upload_session_id: uploadId } } })) },
  async getBuckets() { const [buckets, media] = await backendBucketsAndMedia(); return buckets.map((bucket) => bucketFromBackend(bucket, media)) },
  async createBucket(input) { const value = await backendData(backendClient.POST('/api/v1/buckets', { body: { name: input.name, visibility: input.visibility, default_ttl_seconds: input.defaultTtlSeconds ?? undefined, max_object_size: input.maxObjectSize ?? undefined, allowed_mime_types: input.allowedMimeTypes, lifecycle_rules: input.lifecycleRules.map(lifecycleRuleToBackend) } })); return bucketFromBackend(value, []) },
  async updateBucket(name, input) { const value = await backendData(backendClient.PATCH('/api/v1/buckets/{name}', { params: { path: { name } }, body: { visibility: input.visibility, default_ttl_seconds: input.defaultTtlSeconds, max_object_size: input.maxObjectSize, allowed_mime_types: input.allowedMimeTypes, lifecycle_rules: input.lifecycleRules.map(lifecycleRuleToBackend) } })); return bucketFromBackend(value, []) },
  async deleteBucket(name) { await backendOk(backendClient.DELETE('/api/v1/buckets/{name}', { params: { path: { name } } })) },
  async getAccessKeys(appId) { return (await backendData(backendClient.GET('/api/v1/applications/{app_id}/access-keys', { params: { path: { app_id: appId } } }))).map(accessKeyFromBackend) },
  async createAccessKey(appId, input) { const value = await backendData(backendClient.POST('/api/v1/applications/{app_id}/access-keys', { params: { path: { app_id: appId } }, body: { name: input.name, permissions: input.permissions, expires_at: input.expiresAt } })); return { title: '访问密钥已创建', identifier: value.access_key_id, secret: value.secret_access_key } },
  async updateAccessKey(id, input) { return accessKeyFromBackend(await backendData(backendClient.PATCH('/api/v1/access-keys/{access_key_id}', { params: { path: { access_key_id: id } }, body: { name: input.name, permissions: input.permissions, expires_at: input.expiresAt } }))) },
  async rotateAccessKey(appId, key) { const secret = await backendApi.createAccessKey(appId, { name: `${key.name}-rotated`, permissions: key.permissions, expiresAt: key.expiresAt }); try { await backendApi.revokeAccessKey(key.id); return { ...secret, title: '访问密钥已轮换' } } catch { return { ...secret, title: '新密钥已创建；旧密钥撤销失败，请手动撤销' } } },
  async revokeAccessKey(id) { await backendOk(backendClient.DELETE('/api/v1/access-keys/{access_key_id}', { params: { path: { access_key_id: id } } })) },
  async getWebhooks() { return (await backendData(backendClient.GET('/api/v1/webhooks'))).map(webhookFromBackend) },
  async getWebhookDeliveries(webhookId, filters = {}) { const page: BackendWebhookDeliveryPage = await backendData(backendClient.GET('/api/v1/webhooks/{webhook_id}/deliveries', { params: { path: { webhook_id: webhookId }, query: { status: filters.status, limit: filters.limit, cursor: filters.cursor } } })); return { items: page.items.map((item: BackendWebhookDelivery) => ({ eventId: item.event_id, endpointId: item.endpoint_id, eventType: item.event_type, attemptCount: item.attempt_count, status: item.status, lastResponseStatus: item.last_response_status ?? null, lastError: item.last_error ?? null, createdAt: item.created_at, updatedAt: item.updated_at, nextAttemptAt: item.next_attempt_at ?? null, deliveredAt: item.delivered_at ?? null, deadLetteredAt: item.dead_lettered_at ?? null, replayCount: item.replay_count, lastReplayedAt: item.last_replayed_at ?? null })), nextCursor: page.next_cursor } },
  async replayWebhookDelivery(webhookId, eventId) { await backendOk(backendClient.POST('/api/v1/webhooks/{webhook_id}/deliveries/{event_id}/replay', { params: { path: { webhook_id: webhookId, event_id: eventId } } })) },
  async createWebhook(input) { const value = await backendData(backendClient.POST('/api/v1/webhooks', { body: input })); return { title: 'Webhook 已创建', identifier: value.endpoint.id, secret: value.secret } },
  async updateWebhook(id, input) { const value = await backendData(backendClient.PATCH('/api/v1/webhooks/{webhook_id}', { params: { path: { webhook_id: id } }, body: input })); return webhookFromBackend(value.endpoint) },
  async rotateWebhookSecret(id) { const value = await backendData(backendClient.PATCH('/api/v1/webhooks/{webhook_id}', { params: { path: { webhook_id: id } }, body: { rotate_secret: true } })); if (!value.secret) throw new ApiRequestError(502, 'invalid_response', '服务端未返回新的 Webhook Secret'); return { title: 'Webhook Secret 已轮换', identifier: value.endpoint.id, secret: value.secret } },
  async deleteWebhook(id) { await backendOk(backendClient.DELETE('/api/v1/webhooks/{webhook_id}', { params: { path: { webhook_id: id } } })) },
}


export const api: Api = backendApi
