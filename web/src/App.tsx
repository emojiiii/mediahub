import { zodResolver } from '@hookform/resolvers/zod'
import {
  Alert,
  Avatar,
  Button,
  Card,
  Checkbox,
  Chip,
  Input,
  Modal as HeroModal,
  ProgressBar,
  Slider,
  Spinner,
  Switch,
  TextArea,
} from '@heroui/react'
import { buttonVariants } from '@heroui/styles'
import { type InfiniteData, useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  Activity,
  AlertCircle,
  ArrowRight,
  Boxes,
  Check,
  ChevronLeft,
  ChevronRight,
  CircleHelp,
  CloudUpload,
  Copy,
  Database,
  FileImage,
  FolderOpen,
  HardDrive,
  Eye,
  KeyRound,
  LayoutDashboard,
  LoaderCircle,
  LogOut,
  Mail,
  Monitor,
  PanelLeft,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Settings,
  ShieldCheck,
  Trash2,
  UploadCloud,
  Webhook,
  X,
} from 'lucide-react'
import { createContext, lazy, Suspense, useCallback, useContext, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react'
import { useForm } from 'react-hook-form'
import { Navigate, NavLink, Route, Routes, useLocation, useNavigate, useParams, useSearchParams } from 'react-router-dom'
import { z } from 'zod'
import {
  api,
  errorMessage,
  type AccessKey,
  type AdminApplication,
  type AdminSystemSettings,
  type Application,
  type AsyncJobView,
  type AuthSession,
  type BatchAction,
  type BatchItemResult,
  type Bucket,
  type BucketInput,
  type LifecycleRule,
  type MediaFilters,
  type MediaPage,
  type MediaStatus,
  type ObjectItem,
  type OneTimeSecret,
  type Permission,
  type VariantParams,
  type WebhookDelivery,
  type WebhookDeliveryStatus,
  type WebhookEndpoint,
  type WebhookInput,
} from './api'
import { ApplicationSwitcher } from './components/ApplicationSwitcher'
import { SelectControl } from './components/SelectControl'
import {
  DEFAULT_UPLOAD_CONCURRENCY,
  UploadScheduler,
  type UploadRunContext,
  type UploadSchedulerSnapshot,
} from './upload-scheduler'

const EnhancedObjectFileViewer = lazy(() => import('./components/ObjectFileViewer'))

const loginSchema = z.object({
  email: z.string().email('请输入有效的邮箱地址'),
  password: z.string().min(1, '请输入密码'),
})
const registerSchema = z.object({
  email: z.string().email('请输入有效的邮箱地址'),
  password: z.string().min(12, '密码至少需要 12 个字符'),
  confirmation: z.string(),
}).refine((value) => value.password === value.confirmation, { path: ['confirmation'], message: '两次输入的密码不一致' })
const forgotPasswordSchema = z.object({ email: z.string().email('请输入有效的邮箱地址') })
const resetPasswordSchema = z.object({
  token: z.string().min(20, 'Token 长度不正确'),
  password: z.string().min(12, '密码至少需要 12 个字符'),
  confirmation: z.string(),
}).refine((value) => value.password === value.confirmation, { path: ['confirmation'], message: '两次输入的密码不一致' })
type LoginValues = z.infer<typeof loginSchema>
type RegisterValues = z.infer<typeof registerSchema>
type ForgotPasswordValues = z.infer<typeof forgotPasswordSchema>
type ResetPasswordValues = z.infer<typeof resetPasswordSchema>
type QueueStatus = 'queued' | 'uploading' | 'verifying' | 'completed' | 'failed' | 'cancelled' | 'expired'
type UploadTask = {
  id: string
  file?: File
  name: string
  objectKey: string
  size: number
  mime: string
  bucket: string
  progress: number
  status: QueueStatus
  uploadId?: string
  scheduleId?: string
  recovered?: boolean
  error?: string
}
type UploadWork = { taskId: string; file: File; bucket: string; objectKey: string; uploadId?: string }

const MAX_OBJECT_KEY_BYTES = 1024

export function normalizeUploadPath(value: string): string {
  return value.trim().replace(/\\/g, '/').split('/').filter(Boolean).join('/')
}

export function buildUploadObjectKey(path: string, fileName: string): string {
  const normalizedPath = normalizeUploadPath(path)
  return normalizedPath ? `${normalizedPath}/${fileName}` : fileName
}

export function uploadPathValidationError(value: string): string | undefined {
  const normalizedPath = normalizeUploadPath(value)
  if (!normalizedPath) return undefined
  if (/[\x00-\x1f\x7f]/.test(normalizedPath)) return '路径不能包含控制字符'
  if (normalizedPath.split('/').some((segment) => segment === '.' || segment === '..')) return '路径不能包含 . 或 .. 段'
  if (new TextEncoder().encode(`${normalizedPath}/x`).byteLength > MAX_OBJECT_KEY_BYTES) return '路径过长，最终 Object Key 不能超过 1024 字节'
  return undefined
}

export function uploadObjectKeyValidationError(value: string): string | undefined {
  if (!value || /[\x00-\x1f\x7f]/.test(value)) return 'Object Key 不能为空或包含控制字符'
  if (value.split('/').some((segment) => segment === '.' || segment === '..')) return 'Object Key 不能包含 . 或 .. 段'
  if (new TextEncoder().encode(value).byteLength > MAX_OBJECT_KEY_BYTES) return 'Object Key 不能超过 1024 字节'
  return undefined
}

type UploadQueueContextValue = {
  openUploadCenter: () => void
}

const UploadQueueContext = createContext<UploadQueueContextValue | null>(null)

function useUploadQueue(): UploadQueueContextValue {
  const context = useContext(UploadQueueContext)
  if (!context) throw new Error('useUploadQueue must be used within UploadQueueProvider')
  return context
}

function UploadQueueProvider({ appId, children }: { appId: string; children: React.ReactNode }) {
  const queryClient = useQueryClient()
  const buckets = useQuery({ queryKey: ['buckets', appId], queryFn: api.getBuckets })
  const [queue, setQueue] = useState<UploadTask[]>([])
  const [selectedBucket, setSelectedBucket] = useState('')
  const [uploadPath, setUploadPath] = useState('')
  const [uploadPathIssue, setUploadPathIssue] = useState<string>()
  const [uploadCenterOpen, setUploadCenterOpen] = useState(false)
  const inputRef = useRef<HTMLInputElement>(null)
  const updateTask = (id: string, update: Partial<UploadTask>) => {
    setQueue((tasks) => tasks.map((task) => task.id === id ? { ...task, ...update } : task))
  }
  const runnerRef = useRef<(input: UploadWork, context: UploadRunContext) => Promise<ObjectItem>>(async () => { throw new Error('upload runner is not ready') })
  const schedulerRef = useRef<UploadScheduler<UploadWork, ObjectItem> | null>(null)
  if (!schedulerRef.current) schedulerRef.current = new UploadScheduler((input, context) => runnerRef.current(input, context), { concurrency: DEFAULT_UPLOAD_CONCURRENCY })
  const scheduler = schedulerRef.current
  const schedulerSnapshot = useSyncExternalStore(
    scheduler.subscribe,
    scheduler.getSnapshot,
    scheduler.getSnapshot,
  )

  useEffect(() => {
    let disposed = false
    const ids = readUploadSessionIds(appId)
    void Promise.allSettled(ids.map((uploadId) => api.getUploadSession(uploadId))).then((results) => {
      if (disposed) return
      const recovered = results.flatMap((result, index) => {
        if (result.status === 'rejected') {
          if (result.reason instanceof Error && 'status' in result.reason && (result.reason as { status?: number }).status === 404) clearRememberedUploadSession(appId, ids[index])
          return []
        }
        const session = result.value
        const status: QueueStatus = session.state === 'pending' ? 'queued' : session.state
        return [{ id: session.uploadId, name: session.objectKey.split('/').filter(Boolean).pop() || session.objectKey, objectKey: session.objectKey, size: session.expectedSize, mime: session.expectedMime, bucket: session.bucketId, progress: session.state === 'completed' ? 100 : 0, status, uploadId: session.uploadId, recovered: true }]
      })
      setQueue((current) => {
        const currentIds = new Set(current.map((task) => task.id))
        return [...recovered.filter((task) => !currentIds.has(task.id)), ...current]
      })
    })
    return () => { disposed = true }
  }, [appId])

  useEffect(() => {
    const available = buckets.data ?? []
    if (available.length && !available.some((bucket) => bucket.name === selectedBucket)) setSelectedBucket(available[0].name)
    if (!available.length && !buckets.isLoading && selectedBucket) setSelectedBucket('')
    if (available.length) {
      const namesById = new Map(available.map((bucket) => [bucket.id, bucket.name]))
      setQueue((tasks) => {
        let changed = false
        const next = tasks.map((task) => {
          const bucketName = task.recovered ? namesById.get(task.bucket) : undefined
          if (!bucketName || bucketName === task.bucket) return task
          changed = true
          return { ...task, bucket: bucketName }
        })
        return changed ? next : tasks
      })
    }
  }, [buckets.data, buckets.isLoading, selectedBucket])

  runnerRef.current = async (input, context) => {
    const { taskId, file, bucket, objectKey, uploadId } = input
    updateTask(taskId, { status: 'queued', progress: 8, error: undefined, file, name: file.name, objectKey, size: file.size, mime: file.type || 'application/octet-stream', bucket, uploadId })
    const options = {
      signal: context.signal,
      onSession: (newUploadId: string) => { rememberUploadSessionId(appId, newUploadId); updateTask(taskId, { uploadId: newUploadId }) },
      onProgress: (stage: 'creating' | 'uploading' | 'verifying') => updateTask(taskId, stage === 'creating' ? { status: 'queued', progress: 12 } : stage === 'uploading' ? { status: 'uploading', progress: 62 } : { status: 'verifying', progress: 88 }),
    }
    const result = uploadId
      ? await api.resumeUpload(uploadId, file, options)
      : await api.uploadFile(file, { ...options, bucket, objectKey })
    updateTask(taskId, { status: 'completed', progress: 100 })
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ['objects', appId] }),
      queryClient.invalidateQueries({ queryKey: ['buckets', appId] }),
      queryClient.invalidateQueries({ queryKey: ['dashboard', appId] }),
    ])
    return result
  }

  useEffect(() => {
    const scheduledById = new Map(schedulerSnapshot.tasks.map((task) => [task.id, task]))
    setQueue((tasks) => tasks.map((task) => {
      const scheduled = task.scheduleId ? scheduledById.get(task.scheduleId) : undefined
      if (scheduled?.state === 'failed') return { ...task, status: 'failed', progress: 100, error: errorMessage(scheduled.error) }
      if (scheduled?.state === 'cancelled') return { ...task, status: 'cancelled', progress: 0 }
      return task
    }))
  }, [schedulerSnapshot])

  const scheduleUpload = (taskId: string, file: File, bucket: string, objectKey: string, uploadId?: string) => {
    const scheduleId = crypto.randomUUID()
    updateTask(taskId, { scheduleId, status: 'queued', progress: 0, error: undefined, file, bucket, objectKey, uploadId })
    scheduler.enqueue(scheduleId, { taskId, file, bucket, objectKey, uploadId })
  }
  const addFiles = (files: FileList | null, bucket: string, path: string) => {
    if (!files?.length || !bucket) return
    const tasks = Array.from(files).map((file) => ({ id: crypto.randomUUID(), file, name: file.name, objectKey: buildUploadObjectKey(path, file.name), size: file.size, mime: file.type || 'application/octet-stream', bucket, progress: 0, status: 'queued' as const }))
    const invalid = tasks.find((task) => uploadObjectKeyValidationError(task.objectKey))
    if (invalid) {
      setUploadPathIssue(`${invalid.name}：${uploadObjectKeyValidationError(invalid.objectKey)}`)
      return
    }
    if (new Set(tasks.map((task) => task.objectKey)).size !== tasks.length) {
      setUploadPathIssue('所选文件会生成重复的 Object Key')
      return
    }
    setUploadPathIssue(undefined)
    setQueue((current) => [...tasks, ...current])
    for (const task of tasks) scheduleUpload(task.id, task.file, task.bucket, task.objectKey)
  }
  const cancelUpload = (id: string) => {
    const task = queue.find((item) => item.id === id)
    if (task?.scheduleId) scheduler.cancel(task.scheduleId)
    if (task?.uploadId) void api.cancelUpload(task.uploadId).catch(() => undefined)
    updateTask(id, { status: 'cancelled', progress: 0 })
  }
  const retryUpload = (id: string) => {
    const task = queue.find((item) => item.id === id)
    if (task?.file) scheduleUpload(id, task.file, task.bucket, task.objectKey, task.uploadId)
  }
  const resumeUpload = (id: string, file: File) => {
    const task = queue.find((item) => item.id === id)
    if (task?.uploadId) scheduleUpload(id, file, task.bucket, task.objectKey, task.uploadId)
  }
  const clearFinishedUploads = () => setQueue((tasks) => tasks.filter((task) => {
    const keep = !['completed', 'cancelled', 'expired'].includes(task.status)
    if (!keep && task.uploadId) clearRememberedUploadSession(appId, task.uploadId)
    return keep
  }))

  const openUploadCenter = useCallback(() => setUploadCenterOpen(true), [])
  const uploadContext = useMemo(() => ({ openUploadCenter }), [openUploadCenter])
  const chooseFiles = () => inputRef.current?.click()
  const uploadFiles = (files: FileList | null) => addFiles(files, selectedBucket, uploadPath)
  const pathError = uploadPathValidationError(uploadPath) ?? uploadPathIssue
  const changeUploadPath = (value: string) => { setUploadPath(value); setUploadPathIssue(undefined) }

  return <>
    <UploadQueueContext.Provider value={uploadContext}>{children}</UploadQueueContext.Provider>
    {queue.length > 0 && !uploadCenterOpen && <UploadCenterLauncher queue={queue} activeCount={schedulerSnapshot.activeCount} pendingCount={schedulerSnapshot.pendingCount} onOpen={openUploadCenter} />}
    {uploadCenterOpen && <UploadCenterModal buckets={buckets.data ?? []} bucketsLoading={buckets.isLoading} bucketError={buckets.error} selectedBucket={selectedBucket} uploadPath={uploadPath} pathError={pathError} inputRef={inputRef} queue={queue} activeCount={schedulerSnapshot.activeCount} pendingCount={schedulerSnapshot.pendingCount} onBucketChange={setSelectedBucket} onPathChange={changeUploadPath} onChooseFiles={chooseFiles} onFiles={uploadFiles} onCancel={cancelUpload} onRetry={retryUpload} onResume={resumeUpload} onClear={clearFinishedUploads} onClose={() => setUploadCenterOpen(false)} />}
  </>
}

function UploadCenterLauncher({ queue, activeCount, pendingCount, onOpen }: { queue: UploadTask[]; activeCount: number; pendingCount: number; onOpen: () => void }) {
  const completed = queue.filter((task) => task.status === 'completed').length
  const failed = queue.filter((task) => task.status === 'failed' || task.status === 'expired').length
  const waitingForFile = queue.filter((task) => (task.status === 'queued' || task.status === 'failed') && task.uploadId && !task.file).length
  const active = queue.filter((task) => ['queued', 'uploading', 'verifying'].includes(task.status) && !(task.uploadId && !task.file)).length
  const cancelled = queue.filter((task) => task.status === 'cancelled').length
  const label = active > 0 ? `上传中 ${completed}/${queue.length}` : waitingForFile > 0 ? `等待继续 ${waitingForFile}` : failed > 0 ? `${failed} 个上传失败` : cancelled > 0 ? `任务已结束 ${completed + cancelled}/${queue.length}` : `上传完成 ${completed}/${queue.length}`
  return <Button variant="secondary" className="fixed bottom-4 right-4 z-30 h-12 max-w-[calc(100vw-2rem)] border border-separator bg-surface px-3 shadow-lg sm:bottom-6 sm:right-6" aria-label="打开上传中心" aria-live="polite" onClick={onOpen}>
    <span className={cn('grid size-8 shrink-0 place-items-center rounded-md', failed > 0 ? 'bg-danger-soft text-danger' : waitingForFile > 0 ? 'bg-[#fffbeb] text-[#a16207]' : active > 0 ? 'bg-accent-soft text-accent' : 'bg-success-soft text-success')}>{active > 0 ? <LoaderCircle className="size-4 animate-spin" /> : failed > 0 ? <AlertCircle className="size-4" /> : waitingForFile > 0 ? <UploadCloud className="size-4" /> : <Check className="size-4" />}</span>
    <span className="min-w-0 text-left"><span className="block truncate text-xs font-semibold text-foreground">{label}</span><span className="block text-[10px] font-normal text-muted">并发 {activeCount} · 等待 {pendingCount}</span></span>
  </Button>
}

function UploadCenterModal({ buckets, bucketsLoading, bucketError, selectedBucket, uploadPath, pathError, inputRef, queue, activeCount, pendingCount, onBucketChange, onPathChange, onChooseFiles, onFiles, onCancel, onRetry, onResume, onClear, onClose }: {
  buckets: Bucket[]
  bucketsLoading: boolean
  bucketError: unknown
  selectedBucket: string
  uploadPath: string
  pathError?: string
  inputRef: React.RefObject<HTMLInputElement | null>
  queue: UploadTask[]
  activeCount: number
  pendingCount: number
  onBucketChange: (bucket: string) => void
  onPathChange: (path: string) => void
  onChooseFiles: () => void
  onFiles: (files: FileList | null) => void
  onCancel: (id: string) => void
  onRetry: (id: string) => void
  onResume: (id: string, file: File) => void
  onClear: () => void
  onClose: () => void
}) {
  const normalizedPath = normalizeUploadPath(uploadPath)
  const targetPath = `${selectedBucket}/${normalizedPath ? `${normalizedPath}/` : ''}`
  return <Modal title="上传对象" onClose={onClose} wide>
    <input ref={inputRef} className="hidden" type="file" multiple onChange={(event) => { onFiles(event.target.files); event.currentTarget.value = '' }} />
    <section aria-label="上传目标" className="grid gap-3 border-b border-separator pb-4 sm:grid-cols-[minmax(9rem,0.7fr)_minmax(0,1.3fr)]">
      <label className="min-w-0"><span className="mb-1.5 block text-xs font-medium text-muted">Bucket</span><SelectControl aria-label="上传目标 Bucket" value={selectedBucket} options={buckets.map((bucket) => ({ value: bucket.name, label: bucket.name }))} onChange={onBucketChange} /></label>
      <label className="min-w-0"><span className="mb-1.5 block text-xs font-medium text-muted">对象路径 <span className="font-normal text-muted/70">可选</span></span><Input fullWidth aria-label="上传目标路径" aria-invalid={Boolean(pathError)} maxLength={1024} placeholder="images/avatars" value={uploadPath} onBlur={() => onPathChange(normalizedPath)} onChange={(event) => onPathChange(event.target.value)} /></label>
      <div className={cn('flex min-w-0 items-center gap-2 text-xs sm:col-span-2', pathError ? 'text-danger' : 'text-muted')}>
        <FolderOpen aria-hidden="true" className="size-3.5 shrink-0" />
        {pathError ? <span role="alert" className="min-w-0 truncate" title={pathError}>{pathError}</span> : <><span className="shrink-0">上传到</span><code className="min-w-0 truncate text-[11px] text-foreground" title={targetPath}>{targetPath}</code></>}
      </div>
    </section>
    <MutationError error={bucketError} />
    <UploadQueueV3 queue={queue} activeCount={activeCount} pendingCount={pendingCount} chooseDisabled={bucketsLoading || !selectedBucket || Boolean(pathError)} onChooseFiles={onChooseFiles} onCancel={onCancel} onRetry={onRetry} onResume={onResume} onClear={onClear} />
    <div className="mt-4 flex justify-end border-t border-separator pt-4"><Button variant="secondary" onClick={onClose}>关闭</Button></div>
  </Modal>
}

const cn = (...classes: Array<string | false | null | undefined>) => classes.filter(Boolean).join(' ')

export const DEFAULT_VARIANT_PARAMS: VariantParams = {
  width: 600,
  height: 600,
  fit: 'cover',
  quality: 80,
  format: 'webp',
  blur: 0,
  crop: 'center',
  background: 'ffffff',
}

const RASTER_IMAGE_MIME_TYPES = new Set([
  'image/bmp',
  'image/gif',
  'image/jpeg',
  'image/png',
  'image/tiff',
  'image/vnd.microsoft.icon',
  'image/webp',
  'image/x-icon',
])

export function isRasterImageMimeType(mimeType: string): boolean {
  return RASTER_IMAGE_MIME_TYPES.has(mimeType.trim().toLowerCase().split(';', 1)[0])
}

export function isValidVariantParams(value: VariantParams): boolean {
  return Number.isInteger(value.width)
    && value.width >= 1
    && value.width <= 4096
    && Number.isInteger(value.height)
    && value.height >= 1
    && value.height <= 4096
    && Number.isInteger(value.quality)
    && value.quality >= 1
    && value.quality <= 100
    && Number.isInteger(value.blur)
    && value.blur >= 0
    && value.blur <= 100
    && ['cover', 'contain', 'inside'].includes(value.fit)
    && ['jpeg', 'png', 'webp'].includes(value.format)
    && ['center', 'top', 'bottom', 'left', 'right'].includes(value.crop)
    && /^[a-fA-F0-9]{6}$/.test(value.background)
}
function LinkButton({ to, children, className, variant = 'primary', state }: { to: string; children: React.ReactNode; className?: string; variant?: 'primary' | 'secondary' | 'ghost'; state?: unknown }) {
  return <NavLink to={to} state={state} className={buttonVariants({ variant, className: cn('inline-flex items-center justify-center gap-2', className) })}>{children}</NavLink>
}
const formatBytes = (bytes: number) => {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`
}
const appPath = (appId: string, path: string) => `/app/${appId}/${path}`
export const bucketObjectPath = (appId: string, bucket: string) => `/${encodeURIComponent(appId)}/${encodeURIComponent(bucket)}/`
const uploadSessionStorageKey = (appId: string) => `mediahub.upload-sessions.${appId}`
function readUploadSessionIds(appId: string): string[] {
  try {
    const value: unknown = JSON.parse(sessionStorage.getItem(uploadSessionStorageKey(appId)) ?? '[]')
    return Array.isArray(value) ? value.filter((id): id is string => typeof id === 'string' && id.length > 0) : []
  } catch { return [] }
}
function rememberUploadSessionId(appId: string, uploadId: string): void {
  const ids = new Set(readUploadSessionIds(appId)); ids.add(uploadId)
  sessionStorage.setItem(uploadSessionStorageKey(appId), JSON.stringify([...ids]))
}
function clearRememberedUploadSession(appId: string, uploadId: string): void {
  sessionStorage.setItem(uploadSessionStorageKey(appId), JSON.stringify(readUploadSessionIds(appId).filter((id) => id !== uploadId)))
}

function useCurrentUser() {
  return useQuery({ queryKey: ['auth', 'me'], queryFn: api.getMe, staleTime: Infinity })
}

function Logo() {
  return (
    <div className="flex items-center gap-2.5 text-foreground">
      <span className="grid h-8 w-8 place-items-center bg-accent text-sm font-black shadow-sm" style={{ borderRadius: 5 }}>M</span>
      <span className="text-base font-semibold tracking-tight">MediaHub</span>
    </div>
  )
}

function LoadingScreen() {
  return <div className="grid min-h-screen place-items-center bg-background"><Spinner aria-label="加载中" color="accent" size="lg" /></div>
}

function RequireAuth({ children }: { children: React.ReactNode }) {
  const user = useCurrentUser()
  const location = useLocation()
  if (user.isLoading) return <LoadingScreen />
  if (!user.data) return <Navigate to="/login" replace state={{ from: location.pathname }} />
  return <>{children}</>
}

function RequireAdmin({ children }: { children: React.ReactNode }) {
  const user = useCurrentUser()
  if (user.isLoading) return <LoadingScreen />
  if (!user.data) return <Navigate to="/login" replace state={{ from: '/admin' }} />
  if (user.data.systemRole !== 'admin') return <Navigate to="/" replace />
  return <>{children}</>
}

function LoginPage() {
  const navigate = useNavigate()
  const location = useLocation()
  const queryClient = useQueryClient()
  const form = useForm<LoginValues>({
    resolver: zodResolver(loginSchema),
    defaultValues: { email: '', password: '' },
  })
  const login = useMutation({
    mutationFn: async ({ email, password }: LoginValues) => {
      const user = await api.signIn(email, password)
      const applications = await api.getApplications()
      return { user, applications }
    },
    onSuccess: ({ user, applications }) => {
      queryClient.setQueryData(['auth', 'me'], user)
      queryClient.setQueryData(['applications'], applications)
      const target = (location.state as { from?: string } | null)?.from ?? (applications[0] ? appPath(applications[0].appId, 'dashboard') : '/')
      navigate(target, { replace: true })
    },
  })
  const submit = form.handleSubmit((values) => login.mutate(values))
  return (
    <AuthFormShell eyebrow="MediaHub Console" title="登录控制台" description="使用账号邮箱继续管理媒体资源。">
      <form className="mt-7 space-y-4" onSubmit={submit} noValidate>
        <AuthField label="邮箱" error={form.formState.errors.email?.message}>
          <Input fullWidth type="email" autoComplete="email" {...form.register('email')} />
        </AuthField>
        <AuthField label="密码" error={form.formState.errors.password?.message}>
          <Input fullWidth type="password" autoComplete="current-password" {...form.register('password')} />
        </AuthField>
        <MutationError error={login.error} />
        <Button variant="primary" className="h-11 w-full" type="submit" isDisabled={login.isPending}>
          {login.isPending ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <>登录 <ArrowRight className="h-4 w-4" /></>}
        </Button>
      </form>
      <div className="mt-6 flex flex-wrap items-center justify-between gap-3 border-t border-separator pt-5 text-sm">
        <NavLink className="font-medium text-accent hover:underline" to="/forgot-password">忘记密码？</NavLink>
        <NavLink className="font-medium text-accent hover:underline" to="/register">创建账号</NavLink>
      </div>
    </AuthFormShell>
  )
}

function LogoInverse() {
  return <div className="flex items-center gap-2.5"><span className="grid h-8 w-8 place-items-center bg-accent text-sm font-black text-foreground" style={{ borderRadius: 5 }}>M</span><span className="text-base font-semibold">MediaHub</span></div>
}

function AuthFormShell({ eyebrow, title, description, children }: { eyebrow: string; title: string; description: string; children: React.ReactNode }) {
  return (
    <main className="flex min-h-dvh items-center justify-center bg-background px-4 py-8 sm:px-6 sm:py-12">
      <div className="w-full max-w-md">
        <div className="mb-6 flex justify-center"><Logo /></div>
        <Card className="w-full overflow-hidden" variant="default">
          <Card.Content className="px-5 py-6 sm:px-8 sm:py-8">
            <p className="eyebrow">{eyebrow}</p>
            <h1 className="mt-2 text-2xl font-semibold text-foreground">{title}</h1>
            <p className="mt-2 text-sm leading-6 text-muted">{description}</p>
            {children}
          </Card.Content>
        </Card>
        <div className="mt-5 flex items-center justify-center gap-2 text-xs text-muted">
          <ShieldCheck aria-hidden="true" className="size-3.5 text-success" />
          <span>安全会话由 HttpOnly Cookie 提供保护</span>
        </div>
      </div>
    </main>
  )
}

function RegisterPage() {
  const navigate = useNavigate()
  const form = useForm<RegisterValues>({ resolver: zodResolver(registerSchema), defaultValues: { email: '', password: '', confirmation: '' } })
  const registration = useMutation({ mutationFn: ({ email, password }: RegisterValues) => api.register(email, password) })
  const resend = useMutation({ mutationFn: (email: string) => api.resendVerification(email) })
  if (registration.data) {
    const verificationToken = resend.data?.verificationToken ?? registration.data.verificationToken
    return <AuthFormShell eyebrow="待验证" title="检查你的邮箱" description={`账号 ${registration.data.email} 已创建，验证邮箱后即可登录。`}>
      <Alert className="mt-7" status="accent"><Alert.Indicator><Mail className="size-4" /></Alert.Indicator><Alert.Content><Alert.Title>{resend.data ? '验证说明已重新请求' : '等待邮箱验证'}</Alert.Title><Alert.Description>{resend.data?.message ?? '验证 Token 仅可使用一次，并会在有效期后失效。'}</Alert.Description></Alert.Content></Alert>
      {verificationToken && <DevTokenPanel label="Verification Token" token={verificationToken} onUse={() => navigate(`/verify-email?token=${encodeURIComponent(verificationToken)}`)} />}
      <MutationError error={resend.error} />
      <Button variant="secondary" className="mt-6 w-full" isDisabled={resend.isPending} onClick={() => resend.mutate(registration.data.email)}>{resend.isPending ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}重新发送验证说明</Button>
      <div className="mt-3 grid grid-cols-2 gap-2"><LinkButton variant="ghost" to="/login">返回登录</LinkButton><LinkButton to="/verify-email">输入 Token</LinkButton></div>
    </AuthFormShell>
  }
  return <AuthFormShell eyebrow="创建账号" title="开始使用 MediaHub" description="创建账号并验证邮箱后，即可进入控制台。"><form className="mt-7 space-y-4" noValidate onSubmit={form.handleSubmit((values) => registration.mutate(values))}><AuthField label="邮箱" error={form.formState.errors.email?.message}><Input fullWidth type="email" autoComplete="email" {...form.register('email')} /></AuthField><AuthField label="密码" error={form.formState.errors.password?.message}><Input fullWidth type="password" autoComplete="new-password" {...form.register('password')} /></AuthField><AuthField label="确认密码" error={form.formState.errors.confirmation?.message}><Input fullWidth type="password" autoComplete="new-password" {...form.register('confirmation')} /></AuthField><MutationError error={registration.error} /><Button type="submit" variant="primary" className="h-11 w-full" isDisabled={registration.isPending}>{registration.isPending && <LoaderCircle className="h-4 w-4 animate-spin" />}创建账号</Button></form><p className="mt-6 border-t border-separator pt-5 text-center text-sm text-muted">已有账号？ <NavLink className="font-medium text-accent hover:underline" to="/login">返回登录</NavLink></p></AuthFormShell>
}

function VerifyEmailPage() {
  const [searchParams] = useSearchParams()
  const [token, setToken] = useState(searchParams.get('token') ?? '')
  const verification = useMutation({ mutationFn: () => api.verifyEmail(token.trim()) })
  if (verification.isSuccess) return <AuthFormShell eyebrow="验证成功" title="邮箱已激活" description="账号已准备就绪，现在可以使用注册邮箱和密码登录。"><LinkButton className="mt-7 h-11 w-full" to="/login">前往登录</LinkButton></AuthFormShell>
  return <AuthFormShell eyebrow="邮箱验证" title="验证你的邮箱" description="输入邮件中的一次性 Verification Token。"><form className="mt-7 space-y-4" onSubmit={(event) => { event.preventDefault(); if (token.trim().length >= 20) verification.mutate() }}><AuthField label="Verification Token"><TextArea fullWidth className="min-h-24 font-mono text-xs" autoFocus value={token} onChange={(event) => setToken(event.target.value)} /></AuthField><MutationError error={verification.error} /><Button type="submit" variant="primary" className="h-11 w-full" isDisabled={verification.isPending || token.trim().length < 20}>{verification.isPending && <LoaderCircle className="h-4 w-4 animate-spin" />}验证邮箱</Button></form><LinkButton className="mt-3 w-full" variant="ghost" to="/login">返回登录</LinkButton></AuthFormShell>
}

function ForgotPasswordPage() {
  const navigate = useNavigate()
  const form = useForm<ForgotPasswordValues>({ resolver: zodResolver(forgotPasswordSchema), defaultValues: { email: '' } })
  const request = useMutation({ mutationFn: ({ email }: ForgotPasswordValues) => api.forgotPassword(email) })
  if (request.data) return <AuthFormShell eyebrow="请求已受理" title="检查密码重置说明" description="如果该账号存在，密码重置说明已经发出。"><Alert className="mt-7" status="accent"><Alert.Indicator><Mail className="size-4" /></Alert.Indicator><Alert.Content><Alert.Description>为保护账号隐私，所有邮箱都会看到相同结果。</Alert.Description></Alert.Content></Alert>{request.data.resetToken && <DevTokenPanel label="Reset Token" token={request.data.resetToken} onUse={() => navigate(`/reset-password?token=${encodeURIComponent(request.data.resetToken ?? '')}`)} />}<LinkButton className="mt-6 h-11 w-full" variant="secondary" to="/login">返回登录</LinkButton></AuthFormShell>
  return <AuthFormShell eyebrow="找回账号" title="重置密码" description="输入注册邮箱，我们将发送密码重置说明。"><form className="mt-7 space-y-4" noValidate onSubmit={form.handleSubmit((values) => request.mutate(values))}><AuthField label="邮箱" error={form.formState.errors.email?.message}><Input fullWidth type="email" autoComplete="email" autoFocus {...form.register('email')} /></AuthField><MutationError error={request.error} /><Button type="submit" variant="primary" className="h-11 w-full" isDisabled={request.isPending}>{request.isPending && <LoaderCircle className="h-4 w-4 animate-spin" />}发送重置说明</Button></form><LinkButton className="mt-3 w-full" variant="ghost" to="/login">返回登录</LinkButton></AuthFormShell>
}

function ResetPasswordPage() {
  const [searchParams] = useSearchParams()
  const queryClient = useQueryClient()
  const form = useForm<ResetPasswordValues>({ resolver: zodResolver(resetPasswordSchema), defaultValues: { token: searchParams.get('token') ?? '', password: '', confirmation: '' } })
  const reset = useMutation({ mutationFn: ({ token, password }: ResetPasswordValues) => api.resetPassword(token.trim(), password), onSuccess: () => { queryClient.setQueryData(['auth', 'me'], null); queryClient.removeQueries({ queryKey: ['auth', 'sessions'] }) } })
  if (reset.isSuccess) return <AuthFormShell eyebrow="密码已更新" title="重新登录" description="原有登录会话已撤销，请使用新密码继续。"><LinkButton className="mt-7 h-11 w-full" to="/login">前往登录</LinkButton></AuthFormShell>
  return <AuthFormShell eyebrow="设置新密码" title="重置密码" description="输入重置 Token 并设置新的账号密码。"><form className="mt-7 space-y-4" noValidate onSubmit={form.handleSubmit((values) => reset.mutate(values))}><AuthField label="Reset Token" error={form.formState.errors.token?.message}><TextArea fullWidth className="min-h-20 font-mono text-xs" {...form.register('token')} /></AuthField><AuthField label="新密码" error={form.formState.errors.password?.message}><Input fullWidth type="password" autoComplete="new-password" {...form.register('password')} /></AuthField><AuthField label="确认新密码" error={form.formState.errors.confirmation?.message}><Input fullWidth type="password" autoComplete="new-password" {...form.register('confirmation')} /></AuthField><MutationError error={reset.error} /><Button type="submit" variant="primary" className="h-11 w-full" isDisabled={reset.isPending}>{reset.isPending && <LoaderCircle className="h-4 w-4 animate-spin" />}更新密码</Button></form><LinkButton className="mt-3 w-full" variant="ghost" to="/login">返回登录</LinkButton></AuthFormShell>
}

function AuthField({ label, error, children }: { label: string; error?: string; children: React.ReactNode }) {
  return <label className="block"><span className="mb-2 block text-sm font-medium text-foreground">{label}</span>{children}{error && <span className="mt-1.5 block text-xs text-danger">{error}</span>}</label>
}

function DevTokenPanel({ label, token, onUse }: { label: string; token: string; onUse: () => void }) {
  const [copied, setCopied] = useState(false)
  const copy = async () => { await navigator.clipboard.writeText(token); setCopied(true) }
  return <section className="mt-5 border border-[#e0c78f] bg-[#fff8e8] p-4" style={{ borderRadius: 6 }}><div className="flex items-center justify-between gap-3"><p className="text-xs font-semibold text-[#815d18]">开发环境 {label}</p><Button variant="ghost" className="h-8 w-8 p-0" aria-label="复制 Token" onClick={() => void copy()}>{copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}</Button></div><code className="mt-2 block break-all text-xs leading-5 text-[#604818]">{token}</code><Button variant="primary" className="mt-4 w-full" onClick={onUse}>使用此 Token<ArrowRight className="h-4 w-4" /></Button></section>
}

const primaryNav = [
  { label: '总览', path: 'dashboard', icon: LayoutDashboard },
  { label: '对象', path: 'objects', icon: Boxes },
  { label: 'Buckets', path: 'buckets', icon: Database },
  { label: '访问密钥', path: 'access-keys', icon: KeyRound },
  { label: 'Webhook', path: 'webhooks', icon: Webhook },
]

function ConsoleShellV3() {
  const { appId = '' } = useParams()
  api.setApplication(appId)
  const user = useCurrentUser()
  const applications = useQuery({ queryKey: ['applications'], queryFn: api.getApplications })
  const capabilities = useQuery({ queryKey: ['capabilities'], queryFn: api.getCapabilities, staleTime: 60_000 })
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const [navOpen, setNavOpen] = useState(false)
  const navRef = useRef<HTMLElement>(null)
  const [creatingApplication, setCreatingApplication] = useState(false)
  const [applicationName, setApplicationName] = useState('')
  const selected = applications.data?.find((app) => app.appId === appId)
  const signOut = useMutation({ mutationFn: api.signOut, onSuccess: () => { api.setApplication(undefined); queryClient.setQueryData(['auth', 'me'], null); navigate('/login') } })
  const createApplication = useMutation({
    mutationFn: () => api.createApplication(applicationName.trim()),
    onSuccess: async (application) => {
      await queryClient.invalidateQueries({ queryKey: ['applications'] })
      setCreatingApplication(false)
      setApplicationName('')
      navigate(appPath(application.appId, 'dashboard'))
    },
  })
  useEffect(() => {
    if (!navOpen) return
    const previousOverflow = document.body.style.overflow
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null
    const closeOnEscape = (event: KeyboardEvent) => { if (event.key === 'Escape') setNavOpen(false) }
    document.body.style.overflow = 'hidden'
    document.addEventListener('keydown', closeOnEscape)
    navRef.current?.focus()
    return () => {
      document.body.style.overflow = previousOverflow
      document.removeEventListener('keydown', closeOnEscape)
      previousFocus?.focus()
    }
  }, [navOpen])
  const navigation = <>
    <div className="border-b border-white/10 px-4 py-4"><LogoInverse /></div>
    <div className="px-3 py-4">
      <p className="mb-2 px-1 text-[10px] font-semibold uppercase text-white/35">当前应用</p>
      <ApplicationSwitcher applications={applications.data ?? []} currentAppId={appId} isLoading={applications.isLoading} onSelect={(nextAppId) => { navigate(appPath(nextAppId, 'dashboard')); setNavOpen(false) }} onCreate={() => setCreatingApplication(true)} />
      {applications.error && <p className="mt-2 px-1 text-xs leading-5 text-danger-soft-foreground">{errorMessage(applications.error)}</p>}
    </div>
    <nav className="flex-1 px-3" aria-label="应用导航">
      <p className="mb-2 px-3 text-[10px] font-semibold uppercase text-white/35">工作区</p>
      <div className="space-y-1">{primaryNav.map((item) => <NavItemV3 key={item.path} {...item} appId={appId} onNavigate={() => setNavOpen(false)} />)}</div>
      <p className="mb-2 mt-6 px-3 text-[10px] font-semibold uppercase text-white/35">配置</p>
      <NavItemV3 appId={appId} icon={Settings} label="设置" path="settings" onNavigate={() => setNavOpen(false)} />
      {user.data?.systemRole === 'admin' && <NavLink to="/admin" onClick={() => setNavOpen(false)} className="mt-1 flex h-10 items-center gap-3 rounded-md px-3 text-sm font-medium text-white/60 transition hover:bg-white/[.06] hover:text-white"><ShieldCheck className="size-4" />系统管理</NavLink>}
    </nav>
    <div className="border-t border-white/10 p-3">
      <div className="flex items-center gap-3 rounded-md px-2 py-2"><Avatar size="sm" color="accent"><Avatar.Fallback>{user.data?.name.slice(0, 1).toUpperCase()}</Avatar.Fallback></Avatar><div className="min-w-0 flex-1"><p className="truncate text-sm font-medium text-white">{user.data?.name}</p><p className="truncate text-xs text-white/40">{user.data?.email}</p></div><Button isIconOnly size="sm" variant="ghost" className="text-white/55 hover:text-white" aria-label="退出登录" onClick={() => signOut.mutate()}><LogOut className="size-4" /></Button></div>
    </div>
  </>
  return <div className="min-h-screen bg-background lg:grid lg:grid-cols-[260px_minmax(0,1fr)]">
    {navOpen && <button className="fixed inset-0 z-40 bg-black/35 lg:hidden" aria-label="关闭导航" onClick={() => setNavOpen(false)} />}
    <aside ref={navRef} id="mobile-navigation" tabIndex={-1} role={navOpen ? 'dialog' : undefined} aria-modal={navOpen || undefined} aria-label="应用导航" className={cn('fixed inset-y-0 left-0 z-50 flex w-[288px] flex-col border-r border-white/10 bg-surface text-white outline-none transition-transform lg:w-[260px] lg:translate-x-0', navOpen ? 'translate-x-0' : '-translate-x-full')}>
      {navigation}
    </aside>
    <main className="min-w-0 lg:col-start-2">
      <header className="sticky top-0 z-30 flex h-16 items-center gap-3 border-b border-separator bg-surface/95 px-4 backdrop-blur sm:px-6 lg:px-8">
        <Button isIconOnly size="sm" variant="ghost" className="lg:hidden" aria-label="打开导航" aria-controls="mobile-navigation" aria-expanded={navOpen} onClick={() => setNavOpen(true)}><PanelLeft className="size-4" /></Button>
        <div className="min-w-0 flex-1"><div className="flex items-center gap-2 text-xs text-muted"><span>MediaHub</span><span>/</span><span className="truncate text-foreground">{selected?.name ?? '应用'}</span></div><p className="mt-0.5 truncate font-mono text-[10px] text-muted">{selected?.appId ?? appId}</p></div>
        <Chip size="sm" variant="soft" color="success"><span className="mr-1 size-1.5 rounded-full bg-success" /><Chip.Label>{capabilities.data?.storageBackend === 'local' ? '本地存储' : capabilities.data?.storageBackend ?? '连接中'}</Chip.Label></Chip>
      </header>
      <UploadQueueProvider key={appId} appId={appId}><div className="mx-auto w-full max-w-[1760px] p-4 sm:p-6 lg:p-8"><Routes><Route path="dashboard" element={<DashboardPage />} /><Route path="objects" element={<ObjectsPage />} /><Route path="objects/:mediaId" element={<ObjectDetailPage />} /><Route path="buckets" element={<BucketsPage />} /><Route path="access-keys" element={<AccessKeysPage />} /><Route path="webhooks" element={<WebhooksPage />} /><Route path="settings" element={<SettingsPage />} /><Route path="*" element={<Navigate to="dashboard" replace />} /></Routes></div></UploadQueueProvider>
    </main>
    {creatingApplication && <CreateApplicationModal name={applicationName} pending={createApplication.isPending} error={createApplication.error} onChange={setApplicationName} onClose={() => { createApplication.reset(); setApplicationName(''); setCreatingApplication(false) }} onSubmit={() => createApplication.mutate()} />}
  </div>
}

function CreateApplicationModal({ name, pending, error, onChange, onClose, onSubmit }: { name: string; pending: boolean; error: unknown; onChange: (name: string) => void; onClose: () => void; onSubmit: () => void }) {
  const valid = Boolean(name.trim())
  return <Modal title="新建应用" size="sm" bodyClassName="p-0" dialogClassName="overflow-hidden" dismissable={!pending} showClose={!pending} onClose={onClose}>
    <form onSubmit={(event) => { event.preventDefault(); if (valid && !pending) onSubmit() }}>
      <div className="space-y-5 px-5 pb-5 pt-1">
        <div className="flex items-start gap-3">
          <span className="grid size-10 shrink-0 place-items-center rounded-md bg-accent-soft text-accent"><Boxes className="size-5" /></span>
          <div className="min-w-0 pt-0.5"><p className="text-sm font-semibold text-foreground">独立资源空间</p><p className="mt-1 text-xs leading-5 text-muted">Buckets、对象、访问密钥和 Webhook 将只属于这个应用。</p></div>
        </div>
        <label className="block"><span className="mb-1.5 block text-xs font-medium text-muted">应用名称</span><Input fullWidth autoFocus maxLength={128} placeholder="例如：产品素材库" value={name} onChange={(event) => onChange(event.target.value)} /></label>
        <MutationError error={error} />
      </div>
      <div className="grid grid-cols-2 gap-2 border-t border-separator bg-default-soft px-5 py-4">
        <Button variant="secondary" type="button" isDisabled={pending} onClick={onClose}>取消</Button>
        <Button type="submit" variant="primary" isDisabled={pending || !valid}>{pending ? <LoaderCircle className="size-4 animate-spin" /> : <Plus className="size-4" />}创建应用</Button>
      </div>
    </form>
  </Modal>
}
function NavItemV3({ appId, path, label, icon: Icon, onNavigate }: { appId: string; path: string; label: string; icon: typeof LayoutDashboard; onNavigate: () => void }) {
  return <NavLink to={appPath(appId, path)} onClick={onNavigate} className={({ isActive }) => cn('group relative flex h-10 items-center gap-3 rounded-md px-3 text-sm font-medium transition', isActive ? 'bg-white/10 text-white shadow-[inset_0_0_0_1px_rgb(255_255_255/0.05)]' : 'text-white/60 hover:bg-white/[.06] hover:text-white')}><span className={cn('grid h-6 w-6 place-items-center rounded', 'group-hover:text-white')}><Icon className="size-4" /></span>{label}</NavLink>
}

function DashboardPage() {
  const { appId = '' } = useParams()
  const { openUploadCenter } = useUploadQueue()
  const dashboard = useQuery({ queryKey: ['dashboard', appId], queryFn: () => api.getDashboard(appId) })
  if (dashboard.isLoading || !dashboard.data) return <PageLoading />
  const data = dashboard.data
  const usage = data.app.quotaBytes ? Math.round((data.app.usedBytes / data.app.quotaBytes) * 100) : 0
  return <>
    <section className="overflow-hidden rounded-lg border border-separator bg-surface shadow-sm">
      <div className="flex min-h-14 flex-wrap items-center justify-between gap-3 border-b border-separator px-4 py-2.5 sm:px-5">
        <h1 className="text-sm font-semibold text-foreground">总览</h1>
        <Button variant="primary" className="h-9 px-3.5" isDisabled={!data.buckets.length} onClick={openUploadCenter}><UploadCloud className="h-4 w-4" />上传对象</Button>
      </div>
      <div className="grid sm:grid-cols-2 xl:grid-cols-4">
        <Metric label="对象总数" value={data.objectCount.toLocaleString()} detail={`${data.buckets.length} 个 Bucket`} icon={Boxes} tone="blue" />
        <Metric label="原始文件容量" value={formatBytes(data.app.usedBytes)} detail={`${usage}% 配额已使用`} icon={HardDrive} tone="teal" />
        <Metric label="今日上传" value={data.operationalMetricsAvailable ? data.todayUploads.toString() : '--'} detail={data.operationalMetricsAvailable ? '来自运行指标' : '指标暂未接入'} icon={CloudUpload} tone="amber" />
        <Metric label="今日删除" value={data.operationalMetricsAvailable ? data.todayDeletes.toString() : '--'} detail={data.operationalMetricsAvailable ? '来自运行指标' : '指标暂未接入'} icon={Trash2} tone="rose" />
      </div>
    </section>

    <section className="mt-5 grid gap-5 xl:grid-cols-[minmax(0,1.55fr)_minmax(320px,.7fr)]">
      <Card variant="default" className="overflow-hidden">
        <Card.Header className="flex items-start justify-between border-b border-separator px-5 py-4">
          <div><Card.Title className="text-sm font-semibold">存储水位</Card.Title><Card.Description className="mt-1 text-xs">按 Bucket 汇总已提交对象</Card.Description></div>
          <div className="text-right"><p className="text-sm font-semibold">{formatBytes(data.app.usedBytes)}</p><p className="mt-0.5 text-[11px] text-muted">共 {formatBytes(data.app.quotaBytes)}</p></div>
        </Card.Header>
        <Card.Content className="px-5 py-5">
          <div className="flex items-center gap-3"><div className="h-2 flex-1 overflow-hidden rounded-full bg-default"><div className="h-full rounded-full bg-accent transition-all" style={{ width: `${Math.max(usage, usage > 0 ? 2 : 0)}%` }} /></div><span className="w-10 text-right text-xs font-semibold text-accent">{usage}%</span></div>
          <div className="mt-5 divide-y divide-separator">
            {data.buckets.map((bucket) => <div key={bucket.name} className="grid grid-cols-[minmax(0,1fr)_auto_auto] items-center gap-4 py-3 first:pt-0 last:pb-0"><div className="min-w-0"><p className="truncate text-sm font-medium">{bucket.name}</p><div className="mt-2 h-1 overflow-hidden rounded-full bg-default"><div className="h-full rounded-full bg-[#14b8a6]" style={{ width: `${bucket.share}%` }} /></div></div><span className="text-xs tabular-nums text-muted">{bucket.used}</span><span className="min-w-16 text-right text-xs text-muted">{bucket.objects} 个对象</span></div>)}
          </div>
        </Card.Content>
      </Card>

      <Card variant="default">
        <Card.Header className="border-b border-separator px-5 py-4"><Card.Title className="text-sm font-semibold">文件类型</Card.Title><Card.Description className="mt-1 text-xs">按原始文件容量统计</Card.Description></Card.Header>
        <Card.Content className="px-5 py-5">
          <div className="flex h-2 overflow-hidden rounded-full bg-default">{data.mime.filter((item) => item.amount > 0).map((item) => <span key={item.label} className="h-full" style={{ width: `${item.amount}%`, backgroundColor: item.color }} />)}</div>
          <div className="mt-5 space-y-3">{data.mime.map((item) => <div key={item.label} className="flex items-center gap-3"><i className="h-2.5 w-2.5 rounded-full" style={{ backgroundColor: item.color }} /><span className="flex-1 text-sm text-muted">{item.label}</span><span className="text-sm font-semibold tabular-nums">{item.amount}%</span></div>)}</div>
          {!data.mime.some((item) => item.amount > 0) && <p className="mt-5 border-t border-separator pt-4 text-xs leading-5 text-muted">上传首个对象后，这里会显示容量构成。</p>}
        </Card.Content>
      </Card>
    </section>

    <section className="mt-5 overflow-hidden rounded-lg border border-separator bg-surface shadow-sm"><div className="flex items-center justify-between border-b border-separator px-5 py-4"><div><h2 className="text-sm font-semibold">服务状态</h2><p className="mt-1 text-xs text-muted">来自运行时能力接口</p></div><Activity className="h-4 w-4 text-muted" /></div><div className="grid divide-y divide-separator sm:grid-cols-3 sm:divide-x sm:divide-y-0"><StatusRow label="对象存储" value="已连接" detail={`${data.storageBackend} backend`} /><StatusRow label="Webhook 队列" value="暂无指标" detail="投递指标尚未接入" caution /><StatusRow label="图像处理" value={data.imageProcessing ? '可用' : '不可用'} detail={data.imageProcessing ? 'Variant 处理已启用' : '能力接口未声明'} caution={!data.imageProcessing} /></div></section>
  </>
}

export function buildMimeGradient(items: Array<{ amount: number; color: string }>): string {
  let cursor = 0
  const segments = items.map((item) => { const start = cursor; cursor += Math.max(0, item.amount); return `${item.color} ${start}% ${Math.min(100, cursor)}%` })
  return `conic-gradient(${segments.join(', ') || '#e6e7df 0% 100%'})`
}

function Metric({ label, value, detail, icon: Icon, tone }: { label: string; value: string; detail: string; icon: typeof Boxes; tone: 'blue' | 'teal' | 'amber' | 'rose' }) {
  const tones = { blue: 'bg-[#eff6ff] text-[#2563eb]', teal: 'bg-[#ecfdf5] text-[#0f766e]', amber: 'bg-[#fffbeb] text-[#b45309]', rose: 'bg-[#fff1f2] text-[#be123c]' }
  return <article className="border-b border-separator p-5 last:border-b-0 sm:[&:nth-child(2)]:border-b-0 sm:[&:nth-child(odd)]:border-r xl:border-b-0 xl:border-r xl:last:border-r-0"><div className="flex items-center justify-between gap-3"><p className="text-xs font-medium text-muted">{label}</p><span className={cn('grid h-8 w-8 place-items-center rounded-md', tones[tone])}><Icon className="h-4 w-4" /></span></div><p className="mt-4 text-2xl font-semibold tabular-nums text-foreground">{value}</p><p className="mt-1 text-xs text-muted">{detail}</p></article>
}
function StatusRow({ label, value, detail, caution }: { label: string; value: string; detail: string; caution?: boolean }) {
  return <div className="flex items-start gap-3 px-5 py-4"><span className={cn('mt-1.5 h-2 w-2 shrink-0 rounded-full', caution ? 'bg-[#f59e0b]' : 'bg-[#10b981]')} /><div><p className="text-sm font-medium">{label}</p><p className={cn('mt-1 text-xs font-semibold', caution ? 'text-[#a16207]' : 'text-[#047857]')}>{value}</p><p className="mt-0.5 text-xs text-muted">{detail}</p></div></div>
}
function PageLoading() { return <div className="grid min-h-[420px] place-items-center"><Spinner aria-label="加载中" color="accent" /></div> }

export function objectListRefetchInterval(data: { pages: Array<{ items: Array<Pick<ObjectItem, 'status'>> }> } | undefined): number | false {
  return data?.pages.some((page) => page.items.some((item) => item.status === 'delete_pending')) ? 2000 : false
}

export const DEFAULT_OBJECT_FILTERS: MediaFilters = { limit: 25, status: 'active' }

export function normalizeDirectoryPrefix(value: string | undefined): string {
  const segments = (value ?? '').trim().replace(/\\/g, '/').split('/').filter(Boolean)
  return segments.length ? `${segments.join('/')}/` : ''
}

export function directoryBreadcrumbs(prefix: string): Array<{ label: string; prefix: string }> {
  const segments = normalizeDirectoryPrefix(prefix).split('/').filter(Boolean)
  return segments.map((label, index) => ({ label, prefix: `${segments.slice(0, index + 1).join('/')}/` }))
}

type ObjectListNavigationState = {
  from: string
  objectList?: {
    filters: MediaFilters
    filterDraft: MediaFilters
  }
}

function getObjectListNavigationState(value: unknown): ObjectListNavigationState | null {
  if (!value || typeof value !== 'object') return null
  const candidate = value as { from?: unknown; objectList?: unknown }
  if (typeof candidate.from !== 'string') return null
  if (!candidate.objectList || typeof candidate.objectList !== 'object') return { from: candidate.from }
  const objectList = candidate.objectList as { filters?: unknown; filterDraft?: unknown }
  if (!objectList.filters || typeof objectList.filters !== 'object' || !objectList.filterDraft || typeof objectList.filterDraft !== 'object') {
    return { from: candidate.from }
  }
  return {
    from: candidate.from,
    objectList: {
      filters: objectList.filters as MediaFilters,
      filterDraft: objectList.filterDraft as MediaFilters,
    },
  }
}

function objectListReturnPath(appId: string, from?: string): string {
  const basePath = appPath(appId, 'objects')
  return from === basePath || from?.startsWith(basePath + '?') ? from : basePath
}
export function removeObjectIdsFromPages(
  data: InfiniteData<MediaPage, string> | undefined,
  ids: Iterable<string>,
): InfiniteData<MediaPage, string> | undefined {
  if (!data) return data
  const removedIds = new Set(ids)
  if (removedIds.size === 0) return data
  return {
    ...data,
    pages: data.pages.map((page) => ({ ...page, items: page.items.filter((item) => !removedIds.has(item.id)) })),
  }
}

function ObjectsPage() {
  const { appId = '' } = useParams()
  const location = useLocation()
  const queryClient = useQueryClient()
  const { openUploadCenter } = useUploadQueue()
  const navigationState = getObjectListNavigationState(location.state)
  const restoredObjectList = navigationState?.objectList
  const [filterDraft, setFilterDraft] = useState<MediaFilters>(() => restoredObjectList?.filterDraft ?? { ...DEFAULT_OBJECT_FILTERS })
  const [filters, setFilters] = useState<MediaFilters>(() => restoredObjectList?.filters ?? { ...DEFAULT_OBJECT_FILTERS })
  const [pageIndex, setPageIndex] = useState(0)
  const autoSelectedBucketAppRef = useRef<string | null>(null)
  const directoryMode = Boolean(filters.bucket)
  const objectQueryFilters: MediaFilters = { ...filters, delimiter: directoryMode ? '/' : undefined }
  const items = useInfiniteQuery({
    queryKey: ['objects', appId, objectQueryFilters],
    initialPageParam: '',
    queryFn: ({ pageParam }) => api.getObjects({ ...objectQueryFilters, cursor: pageParam || undefined }),
    getNextPageParam: (page) => page.nextCursor ?? undefined,
    refetchInterval: (query) => objectListRefetchInterval(query.state.data),
  })
  const buckets = useQuery({ queryKey: ['buckets', appId], queryFn: api.getBuckets })
  const [selectedIds, setSelectedIds] = useState<string[]>([])
  useEffect(() => {
    if (!buckets.data || autoSelectedBucketAppRef.current === appId) return
    autoSelectedBucketAppRef.current = appId
    if (buckets.data.length !== 1 || filters.bucket || filterDraft.bucket) return
    const bucket = buckets.data[0]?.name
    if (!bucket) return
    setFilterDraft((current) => ({ ...current, bucket, prefix: undefined }))
    setFilters((current) => ({ ...current, bucket, prefix: undefined }))
    setPageIndex(0)
    setSelectedIds([])
  }, [appId, buckets.data, filterDraft.bucket, filters.bucket])
  const [batchEditorOpen, setBatchEditorOpen] = useState(false)
  const [batchDeleteOpen, setBatchDeleteOpen] = useState(false)
  const [previewItem, setPreviewItem] = useState<ObjectItem | null>(null)
  const [editorItem, setEditorItem] = useState<ObjectItem | null>(null)
  const [deleteItem, setDeleteItem] = useState<ObjectItem | null>(null)
  const [batchResults, setBatchResults] = useState<BatchItemResult[] | null>(null)
  const [activeJobId, setActiveJobId] = useState<string | null>(null)
  const removeFromCachedObjectPages = (ids: string[]) => {
    queryClient.setQueriesData<InfiniteData<MediaPage, string>>(
      { queryKey: ['objects', appId] },
      (data) => removeObjectIdsFromPages(data, ids),
    )
  }
  const refreshResources = () => Promise.all([
    queryClient.invalidateQueries({ queryKey: ['objects', appId] }),
    queryClient.invalidateQueries({ queryKey: ['buckets', appId] }),
    queryClient.invalidateQueries({ queryKey: ['dashboard', appId] }),
  ])
  const batch = useMutation({
    mutationFn: (action: BatchAction) => api.executeBatch(selectedIds, action),
    onSuccess: async (result, action) => {
      const affectedIds = [...selectedIds]
      if (action.type === 'delete') removeFromCachedObjectPages(affectedIds)
      setBatchEditorOpen(false)
      setBatchDeleteOpen(false)
      setBatchResults(null)
      setSelectedIds([])
      if (result.mode === 'job') setActiveJobId(result.job.id)
      else {
        setBatchResults(result.items)
        await refreshResources()
      }
    },
  })
  const remove = useMutation<void, Error, ObjectItem>({
    mutationFn: (item) => api.deleteObject(item.id),
    onSuccess: async (_, item) => {
      setDeleteItem(null)
      setSelectedIds((current) => current.filter((id) => id !== item.id))
      removeFromCachedObjectPages([item.id])
      await refreshResources()
    },
  })
  const batchFinished = async () => {
    setSelectedIds([])
    await refreshResources()
  }
  const pages = items.data?.pages ?? []
  const safePageIndex = Math.min(pageIndex, Math.max(0, pages.length - 1))
  const visibleItems = pages[safePageIndex]?.items ?? []
  const visiblePrefixes = pages[safePageIndex]?.commonPrefixes ?? []
  const currentPrefix = directoryMode ? filters.prefix ?? '' : ''
  const hasCustomObjectFilters = Object.entries(filters).some(([key, value]) => key !== 'limit' && key !== 'status' && Boolean(value))
  const applyFilters = () => {
    const dateValue = (value: string | undefined) => value ? new Date(`${value}T00:00:00`).toISOString() : undefined
    const bucket = filterDraft.bucket?.trim() || undefined
    const prefix = bucket ? normalizeDirectoryPrefix(filterDraft.prefix) || undefined : filterDraft.prefix?.trim() || undefined
    setFilterDraft((current) => ({ ...current, bucket, prefix }))
    setFilters({ ...filterDraft, bucket, prefix, delimiter: undefined, createdFrom: dateValue(filterDraft.createdFrom), createdBefore: dateValue(filterDraft.createdBefore) })
    setPageIndex(0)
    setSelectedIds([])
  }
  const resetFilters = () => { const reset = { ...DEFAULT_OBJECT_FILTERS, limit: filters.limit ?? 25 }; setFilterDraft(reset); setFilters(reset); setPageIndex(0); setSelectedIds([]) }
  const changePageSize = (limit: number) => {
    setFilterDraft((current) => ({ ...current, limit }))
    setFilters((current) => ({ ...current, limit }))
    setPageIndex(0)
    setSelectedIds([])
  }
  const goToPreviousPage = () => setPageIndex((current) => Math.max(0, current - 1))
  const openDirectory = (prefix: string) => {
    const nextPrefix = normalizeDirectoryPrefix(prefix) || undefined
    setFilterDraft((current) => ({ ...current, bucket: filters.bucket, prefix: nextPrefix }))
    setFilters((current) => ({ ...current, prefix: nextPrefix }))
    setPageIndex(0)
    setSelectedIds([])
  }
  const goToNextPage = async () => {
    if (safePageIndex < pages.length - 1) {
      setPageIndex(safePageIndex + 1)
      return
    }
    if (!items.hasNextPage) return
    const result = await items.fetchNextPage()
    const nextIndex = Math.max(0, (result.data?.pages.length ?? pages.length) - 1)
    if (nextIndex > safePageIndex) setPageIndex(nextIndex)
  }
  return <>
    {activeJobId && <JobStatusPanel jobId={activeJobId} onFinished={() => void batchFinished()} onClose={() => setActiveJobId(null)} />}
    {batchResults && <BatchResultsPanel items={batchResults} onClose={() => setBatchResults(null)} />}
    <MutationError error={items.error ?? buckets.error} />
    <section data-testid="object-workspace" className="overflow-hidden rounded-lg border border-separator bg-surface shadow-sm md:flex md:h-[calc(100dvh-7rem)] md:min-h-[560px] md:flex-col lg:h-[calc(100dvh-8rem)]">
      <MediaFilterBar value={filterDraft} buckets={buckets.data ?? []} fetching={items.isFetching} uploadDisabled={buckets.isLoading || !buckets.data?.length} selectedCount={selectedIds.length} batchPending={batch.isPending} onChange={setFilterDraft} onApply={applyFilters} onReset={resetFilters} onRefresh={() => items.refetch()} onUpload={openUploadCenter} onClearSelection={() => setSelectedIds([])} onBatchEdit={() => { batch.reset(); setBatchEditorOpen(true) }} onBatchDelete={() => { batch.reset(); setBatchDeleteOpen(true) }} />
      {directoryMode && filters.bucket && <DirectoryBreadcrumbs bucket={filters.bucket} prefix={currentPrefix} onNavigate={openDirectory} />}
      <div data-testid="object-scroll-region" className="md:min-h-0 md:flex-1 md:overflow-hidden">{items.isLoading ? <PageLoading /> : visibleItems.length || visiblePrefixes.length ? <ObjectTable items={visibleItems} prefixes={visiblePrefixes} currentPrefix={currentPrefix} directoryMode={directoryMode} bucket={filters.bucket ?? ''} appId={appId} selectedIds={selectedIds} deletingId={remove.isPending ? deleteItem?.id : undefined} navigationState={{ from: location.pathname + location.search, objectList: { filters, filterDraft } }} onOpenFolder={openDirectory} onSelectionChange={setSelectedIds} onPreview={setPreviewItem} onEdit={setEditorItem} onDelete={(item) => { remove.reset(); setDeleteItem(item) }} /> : <EmptyState icon={directoryMode ? FolderOpen : Search} title={directoryMode ? '目录为空' : '没有对象'} description={directoryMode ? '当前目录还没有文件或子目录。' : '当前过滤条件没有匹配结果。'} action={Boolean(buckets.data?.length) && (!hasCustomObjectFilters || directoryMode) ? <Button variant="primary" onClick={openUploadCenter}><UploadCloud className="h-4 w-4" />上传对象</Button> : undefined} />}</div>
      {!items.isLoading && <ObjectPagination currentPage={safePageIndex + 1} fetching={items.isFetchingNextPage} hasNext={safePageIndex < pages.length - 1 || Boolean(items.hasNextPage)} hasPrevious={safePageIndex > 0} itemCount={visibleItems.length + visiblePrefixes.length} pageSize={filters.limit ?? 25} onNext={() => void goToNextPage()} onPageSizeChange={changePageSize} onPrevious={goToPreviousPage} />}
    </section>
    {batchEditorOpen && <BatchEditModal selectedCount={selectedIds.length} pending={batch.isPending} error={batch.error} onClose={() => setBatchEditorOpen(false)} onExecute={(action) => batch.mutate(action)} />}
    {batchDeleteOpen && <DeleteObjectsModal count={selectedIds.length} pending={batch.isPending} error={batch.error} onClose={() => setBatchDeleteOpen(false)} onConfirm={() => batch.mutate({ type: 'delete' })} />}
    {previewItem && <ObjectPreviewModal item={previewItem} onClose={() => setPreviewItem(null)} onEdit={() => { setPreviewItem(null); setEditorItem(previewItem) }} />}
    {editorItem && <ObjectEditorModal item={editorItem} onClose={() => setEditorItem(null)} onSaved={async () => { setEditorItem(null); await refreshResources() }} />}
    {deleteItem && <DeleteObjectsModal item={deleteItem} count={1} pending={remove.isPending} error={remove.error} onClose={() => setDeleteItem(null)} onConfirm={() => remove.mutate(deleteItem)} />}
  </>
}

function MediaFilterBar({ value, buckets, fetching, uploadDisabled, selectedCount, batchPending, onChange, onApply, onReset, onRefresh, onUpload, onClearSelection, onBatchEdit, onBatchDelete }: { value: MediaFilters; buckets: Bucket[]; fetching: boolean; uploadDisabled: boolean; selectedCount: number; batchPending: boolean; onChange: (value: MediaFilters) => void; onApply: () => void; onReset: () => void; onRefresh: () => void; onUpload: () => void; onClearSelection: () => void; onBatchEdit: () => void; onBatchDelete: () => void }) {
  const update = <K extends keyof MediaFilters>(key: K, next: MediaFilters[K]) => onChange({ ...value, [key]: next || undefined })
  const invalidRange = Boolean(value.createdFrom && value.createdBefore && value.createdFrom >= value.createdBefore)
  return <div className="shrink-0 border-b border-separator bg-surface px-4 py-4">
    <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-12">
      <label className="xl:col-span-3"><span className="mb-1 block text-xs text-muted">Bucket</span><SelectControl aria-label="Bucket 过滤" value={value.bucket ?? ''} options={[{ value: '', label: '全部 Bucket' }, ...buckets.map((bucket) => ({ value: bucket.name, label: bucket.name }))]} onChange={(next) => onChange({ ...value, bucket: next || undefined, prefix: next === value.bucket ? value.prefix : undefined })} /></label>
      <label className="xl:col-span-3"><span className="mb-1 block text-xs text-muted">状态</span><SelectControl aria-label="状态过滤" value={value.status ?? ''} options={[{ value: '', label: '全部状态' }, { value: 'uploading', label: 'Uploading' }, { value: 'active', label: 'Active' }, { value: 'archive_pending', label: 'Archive pending' }, { value: 'archived', label: 'Archived' }, { value: 'delete_pending', label: 'Delete pending' }, { value: 'deleted', label: 'Deleted' }, { value: 'quarantined', label: 'Quarantined' }]} onChange={(next) => update('status', next as MediaStatus | undefined)} /></label>
      <label className="xl:col-span-3"><span className="mb-1 block text-xs text-muted">MIME</span><Input fullWidth placeholder="image/png" value={value.mime ?? ''} onChange={(event) => update('mime', event.target.value)} /></label>
      <label className="xl:col-span-3"><span className="mb-1 block text-xs text-muted">{value.bucket ? '目录路径' : 'Object Key Prefix'}</span><Input fullWidth placeholder={value.bucket ? 'images/avatar/' : 'campaigns/2026/'} value={value.prefix ?? ''} onChange={(event) => update('prefix', event.target.value)} /></label>
      <label className="xl:col-span-3"><span className="mb-1 block text-xs text-muted">创建不早于</span><Input fullWidth type="date" max={value.createdBefore} value={value.createdFrom ?? ''} onChange={(event) => update('createdFrom', event.target.value)} /></label>
      <label className="xl:col-span-3"><span className="mb-1 block text-xs text-muted">创建早于</span><Input fullWidth type="date" min={value.createdFrom} value={value.createdBefore ?? ''} onChange={(event) => update('createdBefore', event.target.value)} /></label>
      <div data-testid="object-filter-actions" className="flex min-h-[76px] flex-wrap items-end justify-end gap-2 sm:col-span-2 sm:min-h-10 xl:col-span-6">
        {selectedCount > 0 ? <>
          <div className="flex w-full items-center gap-2 sm:mr-auto sm:w-auto"><Badge tone="positive">{selectedCount} 个已选</Badge><Button variant="ghost" size="sm" onClick={onClearSelection}>取消选择</Button></div>
          <Button variant="secondary" isDisabled={batchPending} onClick={onBatchEdit}><Pencil className="size-4" />批量编辑</Button>
          <Button variant="danger-soft" isDisabled={batchPending} onClick={onBatchDelete}><Trash2 className="size-4" />删除</Button>
        </> : <>
          <Button variant="primary" className="w-full sm:w-auto" isDisabled={uploadDisabled} onClick={onUpload}><UploadCloud className="size-4" />上传对象</Button>
          <Button variant="secondary" className="min-w-28" isDisabled={invalidRange} aria-label={invalidRange ? '起始日期必须早于截止日期' : undefined} onClick={onApply}>应用过滤</Button>
          <Button variant="ghost" onClick={onReset}>重置</Button>
          <Button variant="ghost" isIconOnly size="sm" aria-label="刷新列表" onClick={onRefresh}><RefreshCw className={cn('h-4 w-4', fetching && 'animate-spin')} /></Button>
        </>}
      </div>
    </div>
  </div>
}

function DirectoryBreadcrumbs({ bucket, prefix, onNavigate }: { bucket: string; prefix: string; onNavigate: (prefix: string) => void }) {
  const breadcrumbs = directoryBreadcrumbs(prefix)
  const parentPrefix = breadcrumbs.length > 1 ? breadcrumbs[breadcrumbs.length - 2]?.prefix ?? '' : ''
  return <nav aria-label="当前目录" data-testid="directory-breadcrumbs" className="flex min-h-11 shrink-0 items-center gap-1 overflow-x-auto border-b border-separator bg-default-soft px-4 py-2">
    <Button variant="ghost" size="sm" className="shrink-0" onClick={() => onNavigate('')}><HardDrive className="size-4" />{bucket}</Button>
    {breadcrumbs.map((item) => <span key={item.prefix} className="flex shrink-0 items-center gap-1"><ChevronRight className="size-3.5 text-muted" /><Button variant="ghost" size="sm" className="px-2" onClick={() => onNavigate(item.prefix)}>{item.label}</Button></span>)}
    <Button variant="ghost" isIconOnly size="sm" className="ml-auto shrink-0" aria-label="返回上一级目录" isDisabled={!prefix} onClick={() => onNavigate(parentPrefix)}><ChevronLeft className="size-4" /></Button>
  </nav>
}

export function ObjectPagination({ currentPage, fetching, hasNext, hasPrevious, itemCount, pageSize, onNext, onPageSizeChange, onPrevious }: { currentPage: number; fetching: boolean; hasNext: boolean; hasPrevious: boolean; itemCount: number; pageSize: number; onNext: () => void; onPageSizeChange: (value: number) => void; onPrevious: () => void }) {
  return <footer data-testid="object-pagination" className="flex shrink-0 flex-col gap-3 border-t border-separator bg-default-soft px-4 py-3 sm:flex-row sm:items-center sm:justify-between"><p className="text-xs text-muted">本页 {itemCount} 项</p><div className="flex flex-wrap items-center gap-3"><label className="flex items-center gap-2 text-xs text-muted"><span>每页</span><SelectControl aria-label="每页数量" className="w-20" value={String(pageSize)} options={[{ value: '25', label: '25' }, { value: '50', label: '50' }, { value: '100', label: '100' }]} onChange={(next) => onPageSizeChange(Number(next))} /></label><div className="flex h-9 items-center overflow-hidden rounded-md border border-separator bg-surface"><Button variant="ghost" isIconOnly size="sm" className="rounded-none" aria-label="上一页" isDisabled={!hasPrevious || fetching} onClick={onPrevious}><ChevronLeft className="h-4 w-4" /></Button><span className="min-w-20 border-x border-separator px-3 text-center text-xs font-medium tabular-nums">第 {currentPage} 页</span><Button variant="ghost" isIconOnly size="sm" className="rounded-none" aria-label="下一页" isDisabled={!hasNext || fetching} onClick={onNext}>{fetching ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <ChevronRight className="h-4 w-4" />}</Button></div></div></footer>
}

function ObjectTable({ items, prefixes, currentPrefix, directoryMode, bucket, appId, selectedIds, deletingId, navigationState, onOpenFolder, onSelectionChange, onPreview, onEdit, onDelete }: { items: ObjectItem[]; prefixes: string[]; currentPrefix: string; directoryMode: boolean; bucket: string; appId: string; selectedIds: string[]; deletingId?: string; navigationState: ObjectListNavigationState; onOpenFolder: (prefix: string) => void; onSelectionChange: (ids: string[]) => void; onPreview: (item: ObjectItem) => void; onEdit: (item: ObjectItem) => void; onDelete: (item: ObjectItem) => void }) {
  const selected = new Set(selectedIds)
  const allSelected = items.length > 0 && items.every((item) => selected.has(item.id))
  const toggleAll = () => onSelectionChange(allSelected ? selectedIds.filter((id) => !items.some((item) => item.id === id)) : [...new Set([...selectedIds, ...items.map((item) => item.id)])])
  const toggle = (id: string) => onSelectionChange(selected.has(id) ? selectedIds.filter((value) => value !== id) : [...selectedIds, id])
  const checkbox = (label: string, checked: boolean, onChange: () => void) => <Checkbox aria-label={label} isSelected={checked} onChange={onChange}><Checkbox.Content><Checkbox.Control><Checkbox.Indicator /></Checkbox.Control></Checkbox.Content></Checkbox>
  const folderName = (prefix: string) => prefix.slice(currentPrefix.length).replace(/\/$/, '') || prefix.replace(/\/$/, '')
  const actions = (item: ObjectItem) => <div className="flex items-center justify-end gap-1">
    <Button isIconOnly size="sm" variant="ghost" aria-label={`预览 ${item.name}`} onClick={() => onPreview(item)}><Eye className="size-4" /></Button>
    <Button isIconOnly size="sm" variant="ghost" aria-label={`编辑 ${item.name}`} onClick={() => onEdit(item)}><Pencil className="size-4" /></Button>
    <Button isIconOnly size="sm" variant="ghost" className="text-danger" aria-label={`删除 ${item.name}`} isDisabled={deletingId === item.id} onClick={() => onDelete(item)}>{deletingId === item.id ? <LoaderCircle className="size-4 animate-spin" /> : <Trash2 className="size-4" />}</Button>
  </div>
  return <>
    <div data-testid="object-table-scroll" className="hidden h-full overflow-auto overscroll-contain md:block">
      <table className="w-full min-w-[1080px]">
        <thead data-testid="object-table-head" className="sticky top-0 z-10"><tr className="table-head"><th className="w-12 px-4 py-3">{checkbox('选择当前结果', allSelected, toggleAll)}</th><th className="px-5 py-3">对象</th><th className="px-5 py-3">Bucket</th><th className="px-5 py-3">类型</th><th className="px-5 py-3">大小</th><th className="px-5 py-3">创建时间</th><th className="px-5 py-3">状态</th><th className="px-5 py-3">可见性</th><th aria-label="操作" className="w-32 px-3 py-3" /></tr></thead>
        <tbody>{directoryMode && prefixes.map((prefix) => <tr key={`folder:${prefix}`} data-testid="object-folder-row" className="border-b border-separator text-sm transition-colors last:border-0 hover:bg-default-soft" onDoubleClick={() => onOpenFolder(prefix)}>
          <td className="px-4 py-3" />
          <td className="px-5 py-3"><button type="button" aria-label={`打开文件夹 ${folderName(prefix)}`} className="flex min-w-0 items-center gap-3 text-left" onClick={() => onOpenFolder(prefix)}><span className="grid h-9 w-9 shrink-0 place-items-center rounded-md bg-[#fff7d6] text-[#a16207]"><FolderOpen className="h-4 w-4" /></span><span className="min-w-0"><span className="block truncate font-medium text-foreground hover:text-accent">{folderName(prefix)}</span><span className="mt-0.5 block max-w-[260px] truncate text-xs text-muted">{prefix}</span></span></button></td>
          <td className="px-5 py-3 text-muted">{bucket}</td><td className="px-5 py-3 text-muted">文件夹</td><td className="px-5 py-3 text-muted">—</td><td className="px-5 py-3 text-muted">—</td><td className="px-5 py-3 text-muted">—</td><td className="px-5 py-3 text-muted">—</td><td className="px-3 py-3"><div className="flex justify-end"><Button isIconOnly size="sm" variant="ghost" aria-label={`打开文件夹 ${folderName(prefix)}`} onClick={() => onOpenFolder(prefix)}><ChevronRight className="size-4" /></Button></div></td>
        </tr>)}{items.map((item) => <tr key={item.id} className={cn('border-b border-separator text-sm transition-colors last:border-0 hover:bg-default-soft', selected.has(item.id) && 'bg-accent-soft hover:bg-accent-soft')}>
          <td className="px-4 py-3">{checkbox(`选择 ${item.name}`, selected.has(item.id), () => toggle(item.id))}</td>
          <td className="px-5 py-3"><NavLink to={appPath(appId, `objects/${item.id}`)} state={navigationState} className="flex min-w-0 items-center gap-3"><span className="grid h-9 w-9 shrink-0 place-items-center rounded-md bg-[#eff6ff] text-[#2563eb]"><FileImage className="h-4 w-4" /></span><span className="min-w-0"><span className="block truncate font-medium text-foreground hover:text-accent">{item.name}</span><span className="mt-0.5 block max-w-[260px] truncate text-xs text-muted">{item.key}</span></span></NavLink></td>
          <td className="px-5 py-3 text-muted">{item.bucket}</td><td className="px-5 py-3 text-muted">{item.type}</td><td className="px-5 py-3 tabular-nums text-muted">{formatBytes(item.size)}</td><td className="px-5 py-3 text-muted">{formatDateTime(item.createdAt)}</td><td className="px-5 py-3"><Badge tone={item.status === 'active' ? 'positive' : 'warning'}>{item.status}</Badge></td><td className="px-5 py-3"><Badge tone={item.visibility === '公开' ? 'positive' : 'neutral'}>{item.visibility}</Badge></td><td className="px-3 py-3">{actions(item)}</td>
        </tr>)}</tbody>
      </table>
    </div>
    <div className="divide-y divide-separator md:hidden">{directoryMode && prefixes.map((prefix) => <article key={`folder:${prefix}`} data-testid="object-folder-card" className="px-4 py-4" onDoubleClick={() => onOpenFolder(prefix)}>
      <button type="button" aria-label={`打开文件夹 ${folderName(prefix)}`} className="flex w-full items-center gap-3 text-left" onClick={() => onOpenFolder(prefix)}><span className="grid h-10 w-10 shrink-0 place-items-center rounded-md bg-[#fff7d6] text-[#a16207]"><FolderOpen className="size-5" /></span><span className="min-w-0 flex-1"><span className="block truncate text-sm font-medium text-foreground">{folderName(prefix)}</span><span className="mt-1 block truncate font-mono text-[11px] text-muted">{prefix}</span></span><ChevronRight className="size-4 shrink-0 text-muted" /></button>
    </article>)}{items.map((item) => <article key={item.id} className={cn('px-4 py-4', selected.has(item.id) && 'bg-accent-soft')}>
      <div className="flex gap-3"><div className="pt-1">{checkbox(`选择 ${item.name}`, selected.has(item.id), () => toggle(item.id))}</div><NavLink to={appPath(appId, `objects/${item.id}`)} state={navigationState} className="min-w-0 flex-1"><div className="flex items-start justify-between gap-3"><div className="min-w-0"><p className="truncate text-sm font-medium text-foreground">{item.name}</p><p className="mt-1 truncate font-mono text-[11px] text-muted">{item.key}</p></div><Badge tone={item.status === 'active' ? 'positive' : 'warning'}>{item.status}</Badge></div><div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted"><span>{item.bucket}</span><span>{item.type}</span><span>{formatBytes(item.size)}</span><span>{item.visibility}</span></div></NavLink></div>
      <div className="mt-3 border-t border-separator pt-2">{actions(item)}</div>
    </article>)}</div>
  </>
}

function BatchEditModal({ selectedCount, pending, error, onClose, onExecute }: { selectedCount: number; pending: boolean; error: unknown; onClose: () => void; onExecute: (action: BatchAction) => void }) {
  const [operation, setOperation] = useState<'visibility' | 'ttl'>('visibility')
  const [visibility, setVisibility] = useState<'public' | 'private'>('private')
  const [ttl, setTtl] = useState('')
  const [clearTtl, setClearTtl] = useState(false)
  const execute = () => {
    const action: BatchAction = operation === 'visibility' ? { type: 'update_visibility', visibility } : { type: 'update_ttl_seconds', ttl_seconds: clearTtl ? null : Number(ttl) }
    onExecute(action)
  }
  const invalidTtl = operation === 'ttl' && !clearTtl && (!ttl || Number(ttl) < 1)
  return <Modal title="批量编辑对象" onClose={onClose}>
    <form className="space-y-5" onSubmit={(event) => { event.preventDefault(); execute() }}>
      <div className="flex items-center justify-between rounded-md border border-separator bg-default-soft px-4 py-3"><span className="text-sm font-medium">目标对象</span><Badge tone="positive">{selectedCount} 个已选</Badge></div>
      <label><span className="mb-1.5 block text-xs font-medium text-muted">编辑内容</span><SelectControl aria-label="批量编辑内容" value={operation} options={[{ value: 'visibility', label: '可见性' }, { value: 'ttl', label: 'TTL' }]} onChange={(next) => setOperation(next as 'visibility' | 'ttl')} /></label>
      {operation === 'visibility' ? <label><span className="mb-1.5 block text-xs font-medium text-muted">目标可见性</span><SelectControl aria-label="批量目标可见性" value={visibility} options={[{ value: 'private', label: '私有' }, { value: 'public', label: '公开' }]} onChange={(next) => setVisibility(next as 'public' | 'private')} /></label> : <div className="space-y-3"><label><span className="mb-1.5 block text-xs font-medium text-muted">TTL（秒）</span><Input fullWidth type="number" min="1" disabled={clearTtl} placeholder="例如 86400" value={ttl} onChange={(event) => setTtl(event.target.value)} /></label><Switch isSelected={clearTtl} onChange={(selected) => { setClearTtl(selected); if (selected) setTtl('') }}><Switch.Control><Switch.Thumb /></Switch.Control><Switch.Content>清除对象 TTL</Switch.Content></Switch></div>}
      <MutationError error={error} />
      <ModalActions onCancel={onClose} pending={pending} submitLabel={`应用到 ${selectedCount} 个对象`} disabled={invalidTtl} />
    </form>
  </Modal>
}

function DeleteObjectsModal({ item, count, pending, error, onClose, onConfirm }: { item?: ObjectItem; count: number; pending: boolean; error: unknown; onClose: () => void; onConfirm: () => void }) {
  return <Modal title={item ? '删除对象' : '批量删除对象'} onClose={onClose}>
    <div className="space-y-5">
      <div className="rounded-md border border-danger/25 bg-danger-soft px-4 py-4">
        <div className="flex items-start gap-3"><span className="grid size-9 shrink-0 place-items-center rounded-md bg-white/70 text-danger"><Trash2 className="size-4" /></span><div className="min-w-0"><p className="text-sm font-semibold text-danger-soft-foreground">{item ? `删除“${item.name}”` : `删除选中的 ${count} 个对象`}</p><p className="mt-1 text-xs leading-5 text-danger-soft-foreground/80">对象将进入删除流程，此操作不能通过控制台撤销。</p></div></div>
        {item && <code className="mt-3 block truncate rounded bg-white/55 px-3 py-2 text-[11px] text-danger-soft-foreground" title={item.key}>{item.bucket}/{item.key}</code>}
      </div>
      <MutationError error={error} />
      <div className="flex justify-end gap-2 border-t border-separator pt-4"><Button variant="secondary" onClick={onClose}>取消</Button><Button variant="danger-soft" isDisabled={pending} onClick={onConfirm}>{pending ? <LoaderCircle className="size-4 animate-spin" /> : <Trash2 className="size-4" />}确认删除</Button></div>
    </div>
  </Modal>
}

function ObjectEditorModal({ item, onClose, onSaved }: { item: ObjectItem; onClose: () => void; onSaved: () => void | Promise<void> }) {
  const [displayName, setDisplayName] = useState(item.name)
  const [visibility, setVisibility] = useState<'' | 'public' | 'private'>('')
  const [ttl, setTtl] = useState('')
  const [clearTtl, setClearTtl] = useState(false)
  const [metadata, setMetadata] = useState('')
  const update = useMutation({
    mutationFn: async () => {
      let parsed: unknown
      if (metadata.trim()) {
        parsed = JSON.parse(metadata)
        if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed) || Object.keys(parsed as object).some((key) => key !== 'user' && key !== 'ai')) throw new Error('Metadata 只允许 user 和 ai 命名空间')
      }
      await api.updateObject(item.id, item.revision, { displayName: displayName.trim(), visibility: visibility || undefined, ttlSeconds: clearTtl ? null : ttl ? Number(ttl) : undefined, metadata: parsed as { user?: Record<string, unknown>; ai?: Record<string, unknown> } | undefined })
    },
    onSuccess: onSaved,
  })
  return <Modal title="编辑对象" onClose={onClose} wide>
    <form className="space-y-5" onSubmit={(event) => { event.preventDefault(); update.mutate() }}>
      <div className="flex items-center gap-3 rounded-md border border-separator bg-default-soft px-4 py-3"><span className="grid size-9 shrink-0 place-items-center rounded-md bg-[#eff6ff] text-[#2563eb]"><FileImage className="size-4" /></span><div className="min-w-0"><p className="truncate text-sm font-medium">{item.name}</p><code className="mt-0.5 block truncate text-[10px] text-muted" title={item.key}>{item.bucket}/{item.key}</code></div></div>
      <div className="grid gap-4 sm:grid-cols-2"><label className="sm:col-span-2"><span className="mb-1.5 block text-xs font-medium text-muted">显示名称</span><Input fullWidth maxLength={255} value={displayName} onChange={(event) => setDisplayName(event.target.value)} /></label><label><span className="mb-1.5 block text-xs font-medium text-muted">可见性</span><SelectControl aria-label="编辑对象可见性" value={visibility} options={[{ value: '', label: '保持不变' }, { value: 'private', label: '私有' }, { value: 'public', label: '公开' }]} onChange={(next) => setVisibility(next as '' | 'public' | 'private')} /></label><label><span className="mb-1.5 block text-xs font-medium text-muted">TTL（秒）</span><Input fullWidth type="number" min="1" disabled={clearTtl} placeholder="保持不变" value={ttl} onChange={(event) => setTtl(event.target.value)} /></label></div>
      <Switch isSelected={clearTtl} onChange={(selected) => { setClearTtl(selected); if (selected) setTtl('') }}><Switch.Control><Switch.Thumb /></Switch.Control><Switch.Content>清除对象 TTL</Switch.Content></Switch>
      <label><span className="mb-1.5 block text-xs font-medium text-muted">Metadata JSON</span><TextArea fullWidth className="min-h-28 font-mono text-xs" spellCheck={false} placeholder={'{\n  "user": { "project": "launch" }\n}'} value={metadata} onChange={(event) => setMetadata(event.target.value)} /></label>
      <MutationError error={update.error} />
      <ModalActions onCancel={onClose} pending={update.isPending} submitLabel="保存对象" disabled={!displayName.trim()} />
    </form>
  </Modal>
}

type ImagePreviewMode = 'original' | 'variant'
type LoadedVariant = { url: string; expiresAt: string; params: VariantParams }

function ImageVariantToolbar({ mode, params, pending, valid, failed, onModeChange, onParamsChange }: { mode: ImagePreviewMode; params: VariantParams; pending: boolean; valid: boolean; failed: boolean; onModeChange: (mode: ImagePreviewMode) => void; onParamsChange: (params: VariantParams) => void }) {
  return <div data-testid="image-variant-toolbar" className="shrink-0 border-b border-separator bg-surface px-3 py-2">
    <div className="flex min-w-0 items-end gap-3 overflow-x-auto pb-1">
      <div className="shrink-0">
        <span className="mb-1 block text-[10px] font-medium text-muted">预览模式</span>
        <div className="flex rounded-md border border-field-border bg-field p-0.5" role="group" aria-label="图片预览模式">
          <Button size="sm" variant={mode === 'original' ? 'primary' : 'ghost'} className="h-8 min-w-14 px-2.5" onClick={() => onModeChange('original')}>原图</Button>
          <Button size="sm" variant={mode === 'variant' ? 'primary' : 'ghost'} className="h-8 min-w-16 px-2.5" onClick={() => onModeChange('variant')}>Variant</Button>
        </div>
      </div>
      <VariantParameterFields compact value={params} onChange={onParamsChange} />
      <div className="flex h-10 shrink-0 items-center pb-0.5">
        {mode === 'original' ? <span className="whitespace-nowrap text-[10px] text-muted">显示原图</span> : pending ? <span className="flex items-center gap-1.5 whitespace-nowrap text-[10px] text-muted"><LoaderCircle className="size-3.5 animate-spin" />生成中</span> : <span className={cn('whitespace-nowrap text-[10px]', valid && !failed ? 'text-success' : 'text-danger')}>{!valid ? '参数无效' : failed ? '生成失败' : '参数已应用'}</span>}
      </div>
    </div>
  </div>
}

export function ObjectPreviewModal({ item, variant, onClose, onEdit }: { item: ObjectItem; variant?: VariantParams; onClose: () => void; onEdit?: () => void }) {
  const rasterImage = isRasterImageMimeType(item.type)
  const [imageMode, setImageMode] = useState<ImagePreviewMode>(variant ? 'variant' : 'original')
  const [variantParams, setVariantParams] = useState<VariantParams>(() => ({ ...(variant ?? DEFAULT_VARIANT_PARAMS) }))
  const [debouncedVariant, setDebouncedVariant] = useState<VariantParams | null>(null)
  const [loadedVariant, setLoadedVariant] = useState<LoadedVariant | null>(null)
  const [variantImageError, setVariantImageError] = useState<string | null>(null)
  const [variantImageAttempt, setVariantImageAttempt] = useState(0)
  const variantValid = isValidVariantParams(variantParams)
  const variantParamsKey = JSON.stringify(variantParams)
  const debouncedVariantKey = debouncedVariant ? JSON.stringify(debouncedVariant) : ''

  useEffect(() => {
    if (!rasterImage || imageMode !== 'variant' || !variantValid) {
      setDebouncedVariant(null)
      return
    }
    const timeout = window.setTimeout(() => setDebouncedVariant({ ...variantParams }), 350)
    return () => window.clearTimeout(timeout)
  }, [imageMode, rasterImage, variantParamsKey, variantValid])

  const originalPreview = useQuery({
    queryKey: ['objects', item.id, 'signed-preview', 'original'],
    queryFn: () => api.getSignedUrl(item.id),
    staleTime: 4 * 60_000,
    retry: false,
  })
  const variantPreview = useQuery({
    queryKey: ['objects', item.id, 'signed-preview', 'variant', debouncedVariantKey],
    queryFn: () => api.getVariantUrl(item.id, debouncedVariant as VariantParams),
    enabled: rasterImage && imageMode === 'variant' && Boolean(debouncedVariant),
    staleTime: 4 * 60_000,
    retry: false,
  })
  const variantCandidateUrl = variantPreview.data?.url

  useEffect(() => setVariantImageError(null), [variantCandidateUrl])

  const currentVariant = imageMode === 'variant' ? loadedVariant : null
  const currentUrl = rasterImage
    ? currentVariant?.url ?? originalPreview.data?.url
    : originalPreview.data?.url
  const currentExpiresAt = currentVariant?.expiresAt ?? originalPreview.data?.expiresAt
  const variantPending = rasterImage
    && imageMode === 'variant'
    && variantValid
    && (variantParamsKey !== debouncedVariantKey || variantPreview.isFetching || Boolean(variantCandidateUrl && variantCandidateUrl !== loadedVariant?.url))
  const variantError = variantPreview.error ? errorMessage(variantPreview.error) : variantImageError
  const handleVariantParamsChange = (next: VariantParams) => {
    setVariantParams(next)
    setImageMode('variant')
  }
  const retryVariant = () => {
    setVariantImageError(null)
    setVariantImageAttempt((attempt) => attempt + 1)
    void variantPreview.refetch()
  }

  const loading = <div className="flex items-center gap-3 text-sm text-white/65"><Spinner aria-label="加载对象预览" color="accent" /><span>正在加载预览</span></div>
  const originalError = <div className="max-w-md px-6 text-center"><AlertCircle className="mx-auto size-8 text-danger" /><p className="mt-3 text-sm font-medium text-white">预览加载失败</p><p className="mt-1 text-xs leading-5 text-white/50">{errorMessage(originalPreview.error)}</p><Button variant="secondary" className="mt-4" onClick={() => originalPreview.refetch()}><RefreshCw className="size-4" />重试</Button></div>
  const genericMedia = originalPreview.isLoading ? loading : originalPreview.error ? originalError : currentUrl ? <Suspense fallback={<div className="flex items-center gap-3 text-sm text-white/65"><Spinner aria-label="加载多格式查看器" color="accent" /><span>正在加载查看器</span></div>}><EnhancedObjectFileViewer fileName={item.name} mimeType={item.type} size={item.size} url={currentUrl} /></Suspense> : null
  const rasterMedia = <>
    <ImageVariantToolbar mode={imageMode} params={variantParams} pending={variantPending} valid={variantValid} failed={Boolean(variantError)} onModeChange={setImageMode} onParamsChange={handleVariantParamsChange} />
    <div data-testid="raster-preview-viewport" className="relative grid min-h-0 min-w-0 flex-1 place-items-center overflow-hidden p-3 sm:p-5">
      {originalPreview.isLoading ? loading : originalPreview.error ? originalError : currentUrl ? <img data-testid="raster-preview-image" className="block h-full min-h-0 w-full min-w-0 object-contain object-center" src={currentUrl} alt={item.name} /> : null}
      {imageMode === 'variant' && variantCandidateUrl && debouncedVariant && variantCandidateUrl !== loadedVariant?.url && <img
        key={`${variantCandidateUrl}:${variantImageAttempt}`}
        data-testid="variant-image-preloader"
        className="pointer-events-none absolute size-px opacity-0"
        src={variantCandidateUrl}
        alt=""
        aria-hidden="true"
        onLoad={() => setLoadedVariant({ url: variantCandidateUrl, expiresAt: variantPreview.data?.expiresAt ?? '', params: { ...debouncedVariant } })}
        onError={() => setVariantImageError('Variant 图像生成失败，仍保留上一次成功的预览。')}
      />}
      {variantPending && currentUrl && <span className="pointer-events-none absolute right-4 top-4 flex items-center gap-2 rounded-md bg-black/70 px-2.5 py-1.5 text-[10px] text-white shadow-sm"><LoaderCircle className="size-3.5 animate-spin" />正在生成 Variant</span>}
      {imageMode === 'variant' && variantError && <div className="absolute bottom-4 left-4 right-4 flex items-center justify-between gap-3 rounded-md border border-danger/30 bg-danger-soft px-3 py-2 text-xs text-danger-soft-foreground" role="alert"><span>{variantError}</span><Button size="sm" variant="danger-soft" className="shrink-0" onClick={retryVariant}><RefreshCw className="size-3.5" />重试</Button></div>}
    </div>
  </>

  return <Modal title={variant ? 'Variant 预览' : '对象预览'} onClose={onClose} size="cover" containerClassName="h-[calc(100dvh-2rem)] max-h-[calc(100dvh-2rem)] sm:h-[calc(100dvh-3rem)] sm:max-h-[calc(100dvh-3rem)]" dialogClassName="flex h-full min-h-0 flex-col overflow-hidden" bodyClassName="flex min-h-0 flex-1 overflow-hidden p-0">
    <div data-testid="object-preview-layout" className="grid h-full min-h-0 min-w-0 w-full grid-cols-[minmax(0,1fr)] grid-rows-[minmax(0,1fr)_minmax(18rem,42%)] lg:grid-cols-[minmax(0,1fr)_320px] lg:grid-rows-1">
      <div data-testid="object-preview-stage" className={cn('min-h-0 min-w-0 overflow-hidden bg-[#111317]', rasterImage ? 'flex flex-col' : 'grid place-items-center')}>{rasterImage ? rasterMedia : genericMedia}</div>
      <aside className="flex min-h-0 min-w-0 flex-col border-t border-separator bg-surface lg:border-l lg:border-t-0">
        <div className="border-b border-separator p-5"><div className="flex items-start gap-3"><span className="grid size-10 shrink-0 place-items-center rounded-md bg-[#eff6ff] text-[#2563eb]"><FileImage className="size-5" /></span><div className="min-w-0"><h2 className="break-words text-sm font-semibold text-foreground">{item.name}</h2><code className="mt-1 block break-all text-[10px] leading-4 text-muted">{item.key}</code></div></div><div className="mt-4 flex flex-wrap gap-2"><Badge tone={item.status === 'active' ? 'positive' : 'warning'}>{item.status}</Badge><Badge tone={item.visibility === '公开' ? 'positive' : 'neutral'}>{item.visibility}</Badge></div></div>
        <dl data-testid="object-preview-details" className="min-h-0 flex-1 space-y-3 overflow-y-auto p-5 text-sm"><Detail label="Bucket" value={item.bucket} /><Detail label="内容类型" value={currentVariant ? `image/${currentVariant.params.format}` : item.type} /><Detail label="大小" value={formatBytes(item.size)} />{rasterImage && <Detail label="预览模式" value={imageMode === 'variant' ? 'Variant' : '原图'} />}{rasterImage && imageMode === 'variant' && <Detail label="Variant 参数" value={`${variantParams.width} × ${variantParams.height} · ${variantParams.fit} · ${variantParams.format} · Q${variantParams.quality} · Blur ${variantParams.blur}`} />}<Detail label="创建时间" value={formatDateTime(item.createdAt)} /><Detail label="Revision" value={String(item.revision)} /><Detail label="SHA-256" value={item.sha256} mono /></dl>
        <div data-testid="object-preview-actions" className="grid shrink-0 grid-cols-2 gap-2 border-t border-separator p-4">{onEdit && <Button variant="secondary" className={cn('w-full', !currentUrl && 'col-span-2')} onClick={onEdit}><Pencil className="size-4" />编辑对象</Button>}{currentUrl && <a className={buttonVariants({ variant: 'primary', className: cn('inline-flex w-full items-center justify-center gap-2', !onEdit && 'col-span-2') })} href={currentUrl} target="_blank" rel="noreferrer">在新窗口打开<ArrowRight className="size-4" /></a>}{currentExpiresAt && <p className="col-span-2 pt-1 text-center text-[10px] text-muted">链接有效至 {formatDateTime(currentExpiresAt)}</p>}</div>
      </aside>
    </div>
  </Modal>
}

function BatchResultsPanel({ items, onClose }: { items: BatchItemResult[]; onClose: () => void }) {
  const succeeded = items.filter((item) => item.state === 'succeeded').length
  const failed = items.filter((item) => item.state === 'failed')
  return <Modal title="批量操作结果" onClose={onClose} wide><div className="space-y-4"><div className="flex items-center justify-between rounded-md border border-separator bg-default-soft px-4 py-3"><span className="text-sm font-medium">处理完成</span><span className="text-xs text-muted">成功 {succeeded} · 失败 {failed.length}</span></div><div className="max-h-[50vh] divide-y divide-separator overflow-y-auto rounded-md border border-separator">{items.map((item) => <div className="flex items-center gap-3 px-4 py-3 text-xs" key={item.mediaId}><code className="min-w-0 flex-1 truncate">{item.mediaId}</code><Badge tone={item.state === 'succeeded' ? 'positive' : 'danger'}>{item.state === 'succeeded' ? '成功' : '失败'}</Badge>{item.state === 'failed' && <span className="max-w-56 truncate text-danger" title={item.errorSummary}>{item.errorSummary ?? item.errorCode ?? '操作失败'}</span>}</div>)}</div><div className="flex justify-end"><Button variant="secondary" onClick={onClose}>关闭</Button></div></div></Modal>
}

function JobStatusPanel({ jobId, onFinished, onClose }: { jobId: string; onFinished: () => void; onClose: () => void }) {
  const notified = useRef(false)
  const job = useQuery({ queryKey: ['jobs', jobId], queryFn: () => api.getJob(jobId), refetchInterval: (query) => query.state.data && ['completed', 'failed', 'cancelled'].includes(query.state.data.state) ? false : 1000 })
  const cancel = useMutation({ mutationFn: () => api.cancelJob(jobId), onSuccess: () => job.refetch() })
  const data = job.data
  const terminal = data ? ['completed', 'failed', 'cancelled'].includes(data.state) : false
  useEffect(() => { if (terminal && !notified.current) { notified.current = true; onFinished() } }, [onFinished, terminal])
  const completeItems = data ? data.succeededItems + data.failedItems : 0
  const progress = data?.totalItems ? Math.round(completeItems / data.totalItems * 100) : 0
  const labels: Record<AsyncJobView['state'], string> = { pending: '等待执行', running: '执行中', completed: '已完成', failed: '已失败', cancelled: '已取消' }
  return <Modal title="批量任务" onClose={onClose} wide dismissable={terminal} showClose={terminal}><div className="space-y-4"><div className="flex items-start justify-between gap-4 rounded-md border border-separator bg-default-soft px-4 py-3"><div><div className="flex items-center gap-2"><span className="text-sm font-semibold">后台处理</span>{data && <Badge tone={data.state === 'completed' ? 'positive' : data.state === 'failed' ? 'danger' : 'neutral'}>{labels[data.state]}</Badge>}</div><code className="mt-1 block text-[11px] text-muted">{jobId}</code></div>{!terminal && <Button variant="danger-soft" size="sm" isDisabled={cancel.isPending} onClick={() => cancel.mutate()}>{cancel.isPending && <LoaderCircle className="h-3.5 w-3.5 animate-spin" />}取消任务</Button>}</div><MutationError error={job.error ?? cancel.error} />{job.isLoading ? <div className="grid min-h-32 place-items-center"><LoaderCircle className="h-5 w-5 animate-spin text-accent" /></div> : data && <><div><div className="flex items-center justify-between text-xs text-muted"><span>成功 {data.succeededItems} · 失败 {data.failedItems} · 共 {data.totalItems}</span><span>{progress}%</span></div><div className="mt-2 h-2 overflow-hidden rounded-sm bg-default"><div className={cn('h-full transition-all', data.state === 'failed' ? 'bg-danger' : 'bg-accent')} style={{ width: `${progress}%` }} /></div></div>{data.errorSummary && <p className="text-xs text-danger">{data.errorSummary}</p>}<div className="max-h-52 overflow-y-auto rounded-md border border-separator p-3"><div className="flex flex-wrap gap-1">{data.items.map((item) => <span className={cn('rounded px-2 py-1 font-mono text-[10px]', item.state === 'failed' ? 'bg-danger-soft text-danger-soft-foreground' : item.state === 'succeeded' ? 'bg-success-soft text-success-soft-foreground' : 'bg-default-soft text-muted')} key={item.mediaId} title={item.errorSummary}>{item.mediaId.slice(0, 12)}</span>)}</div></div>{terminal && <div className="flex justify-end"><Button variant="secondary" onClick={onClose}>关闭</Button></div>}</>}</div></Modal>
}

function UploadQueueV3({ queue, activeCount, pendingCount, chooseDisabled, onChooseFiles, onCancel, onRetry, onResume, onClear }: {
  queue: UploadTask[]
  activeCount: number
  pendingCount: number
  chooseDisabled: boolean
  onChooseFiles: () => void
  onCancel: (id: string) => void
  onRetry: (id: string) => void
  onResume: (id: string, file: File) => void
  onClear: () => void
}) {
  const labels: Record<QueueStatus, string> = {
    queued: '等待上传',
    uploading: '正在上传',
    verifying: '正在校验',
    completed: '已完成',
    failed: '上传失败',
    cancelled: '已取消',
    expired: '已过期',
  }
  const finishedCount = queue.filter((task) => ['completed', 'cancelled', 'expired'].includes(task.status)).length
  return (
    <section aria-label="上传任务" className="mt-4 overflow-hidden rounded-lg border border-separator bg-surface">
      <header className="flex flex-wrap items-center justify-between gap-3 border-b border-separator px-4 py-3">
        <div className="flex min-w-0 items-center gap-3">
          <div className="grid size-8 shrink-0 place-items-center rounded-md bg-accent-soft text-accent"><UploadCloud className="size-4" /></div>
          <div><h3 className="text-sm font-semibold text-foreground">上传任务</h3><p className="mt-0.5 text-xs text-muted">已并发 {activeCount} · 等待 {pendingCount} · 共 {queue.length}</p></div>
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          {finishedCount > 0 && <Button variant="ghost" size="sm" onClick={onClear}>清理已结束</Button>}
          <Button variant="primary" size="sm" isDisabled={chooseDisabled} onClick={onChooseFiles}><UploadCloud className="size-4" />选择文件</Button>
        </div>
      </header>
      <div data-testid="upload-task-list" className="max-h-[min(52vh,520px)] divide-y divide-separator overflow-y-auto">
        {queue.length > 0 ? queue.map((task) => {
          const failed = task.status === 'failed' || task.status === 'expired'
          const complete = task.status === 'completed'
          return <div key={task.id} className="grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-3 px-4 py-3">
            <div className={cn('grid size-8 place-items-center rounded-md', failed ? 'bg-danger-soft text-danger' : complete ? 'bg-success-soft text-success' : 'bg-default text-muted')}>
              {complete ? <Check className="size-4" /> : failed ? <AlertCircle className="size-4" /> : <FileImage className="size-4" />}
            </div>
            <div className="min-w-0">
              <div className="flex items-center justify-between gap-3"><p className="truncate text-sm font-medium">{task.name}</p><span className="shrink-0 text-xs text-muted">{formatBytes(task.size)}</span></div>
              <code className="mt-0.5 block truncate text-[10px] text-muted" title={`${task.bucket}/${task.objectKey}`}>{task.bucket}/{task.objectKey}</code>
              <div className="mt-2 flex items-center gap-3">
                <ProgressBar className="min-w-0 flex-1" aria-label={`${task.name} 上传进度`} color={failed ? 'danger' : complete ? 'success' : 'accent'} size="sm" value={task.progress}>
                  <ProgressBar.Track><ProgressBar.Fill /></ProgressBar.Track>
                </ProgressBar>
                <Chip size="sm" variant="soft" color={failed ? 'danger' : complete ? 'success' : 'default'}><Chip.Label>{task.recovered && task.status === 'queued' ? '等待继续' : labels[task.status]}</Chip.Label></Chip>
              </div>
              {task.error && <p className="mt-1 truncate text-xs text-danger" title={task.error}>{task.error}</p>}
            </div>
            <div className="flex items-center gap-1">
              {['uploading', 'queued', 'verifying'].includes(task.status) && <Button isIconOnly size="sm" variant="ghost" aria-label="取消上传" onClick={() => onCancel(task.id)}><X className="size-4" /></Button>}
              {task.status === 'failed' && task.file && <Button isIconOnly size="sm" variant="ghost" aria-label="重试上传" onClick={() => onRetry(task.id)}><RefreshCw className="size-4" /></Button>}
              {(task.status === 'queued' || task.status === 'failed') && task.uploadId && !task.file && <label className="grid size-8 cursor-pointer place-items-center rounded-md text-muted hover:bg-default" title="选择原文件继续"><UploadCloud className="size-4" /><input className="hidden" type="file" accept={task.mime} onChange={(event) => { const file = event.target.files?.[0]; if (file) onResume(task.id, file); event.currentTarget.value = '' }} /></label>}
            </div>
          </div>
        }) : <div className="grid min-h-28 place-items-center px-4 py-7 text-center"><div><FileImage className="mx-auto size-5 text-muted" /><p className="mt-2 text-sm text-muted">暂无上传任务</p></div></div>}
      </div>
    </section>
  )
}

function BucketsPage() {
  const { appId = '' } = useParams()
  const buckets = useQuery({ queryKey: ['buckets', appId], queryFn: api.getBuckets })
  const queryClient = useQueryClient()
  const [editor, setEditor] = useState<Bucket | null | undefined>(undefined)
  const refresh = () => Promise.all([
    queryClient.invalidateQueries({ queryKey: ['buckets', appId] }),
    queryClient.invalidateQueries({ queryKey: ['objects', appId] }),
    queryClient.invalidateQueries({ queryKey: ['dashboard', appId] }),
  ])
  const save = useMutation({
    mutationFn: (input: BucketInput) => editor ? api.updateBucket(editor.name, input) : api.createBucket(input),
    onSuccess: async () => { await refresh(); setEditor(undefined) },
  })
  useEffect(() => { save.reset() }, [editor])
  const remove = useMutation({
    mutationFn: api.deleteBucket,
    onSuccess: refresh,
  })
  return <>
    <MutationError error={buckets.error ?? remove.error} />
    <Card variant="default" className="overflow-hidden">
      <Card.Header className="flex flex-col gap-3 border-b border-separator px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="min-w-0"><Card.Title className="text-sm font-semibold">Buckets</Card.Title><Card.Description className="mt-1 text-xs">存储空间、访问范围与自动清理策略</Card.Description></div>
        <div className="flex items-center gap-2"><Chip size="sm" variant="soft"><Chip.Label>{buckets.data?.length ?? 0} 个</Chip.Label></Chip><Button variant="primary" className="min-w-0 flex-1 sm:flex-none" onClick={() => setEditor(null)}><Plus className="size-4" />新建 Bucket</Button></div>
      </Card.Header>
      <Card.Content className="p-0">
        {buckets.isLoading ? <PageLoading /> : buckets.data?.length ? <div className="overflow-x-auto">
          <table className="w-full min-w-[1040px]">
            <thead className="bg-[#f7f9fc] text-left text-[11px] font-semibold text-muted"><tr><th className="px-5 py-3">名称</th><th className="px-5 py-3">可见性</th><th className="px-5 py-3 text-right">对象数量</th><th className="px-5 py-3 text-right">已用容量</th><th className="px-5 py-3">对象访问前缀</th><th className="px-5 py-3">生命周期</th><th aria-label="操作" className="w-24 px-3 py-3" /></tr></thead>
            <tbody className="divide-y divide-separator">{buckets.data.map((bucket) => <tr className="text-sm transition-colors hover:bg-[#f8fbff]" key={bucket.name}>
              <td className="px-5 py-3.5"><div className="flex items-center gap-3"><span className="grid size-8 shrink-0 place-items-center rounded-md bg-[#eff6ff] text-[#2563eb]"><Database className="size-4" /></span><span className="font-medium text-foreground">{bucket.name}</span></div></td>
              <td className="px-5 py-3.5"><Badge tone={bucket.visibility === '公开' ? 'positive' : 'neutral'}>{bucket.visibility}</Badge></td>
              <td className="px-5 py-3.5 text-right tabular-nums text-muted">{bucket.objectCount.toLocaleString()}</td>
              <td className="px-5 py-3.5 text-right tabular-nums text-muted">{bucket.used}</td>
              <td className="max-w-80 px-5 py-3.5"><BucketObjectPath appId={appId} bucket={bucket.name} /></td>
              <td className="max-w-72 px-5 py-3.5"><span className="block truncate text-muted" title={bucket.lifecycle}>{bucket.lifecycle}</span></td>
              <td className="px-3 py-3.5"><div className="flex justify-end gap-1"><Button isIconOnly size="sm" variant="ghost" aria-label="编辑 Bucket" onClick={() => setEditor(bucket)}><Pencil className="size-4" /></Button><Button isIconOnly size="sm" variant="ghost" className="text-danger" isDisabled={remove.isPending || bucket.objectCount > 0} aria-label={bucket.objectCount ? 'Bucket 非空，不能删除' : '删除 Bucket'} onClick={() => { if (window.confirm(`删除空 Bucket “${bucket.name}”？`)) remove.mutate(bucket.name) }}><Trash2 className="size-4" /></Button></div></td>
            </tr>)}</tbody>
          </table>
        </div> : <EmptyState icon={Database} title="还没有 Bucket" description="创建 Bucket 后即可上传对象。" action={<Button variant="primary" onClick={() => setEditor(null)}><Plus className="size-4" />新建 Bucket</Button>} />}
      </Card.Content>
    </Card>
    {editor !== undefined && <BucketEditor bucket={editor} pending={save.isPending} error={save.error} onClose={() => setEditor(undefined)} onSave={(input) => save.mutate(input)} />}
  </>
}

export function BucketObjectPath({ appId, bucket }: { appId: string; bucket: string }) {
  const [copied, setCopied] = useState(false)
  const path = bucketObjectPath(appId, bucket)
  const copy = async () => {
    await navigator.clipboard.writeText(path)
    setCopied(true)
  }
  return <div className="flex min-w-0 items-center gap-1"><code className="block min-w-0 flex-1 truncate text-[11px] text-muted" title={`${path}{object_key}`}>{path}<span className="text-muted/60">object-key</span></code><Button isIconOnly size="sm" variant="ghost" aria-label={`复制 ${bucket} 对象访问前缀`} onClick={() => void copy()}>{copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}</Button></div>
}

const utf8Length = (value: string) => new TextEncoder().encode(value).length
const lifecycleRulesValid = (rules: LifecycleRule[]) => {
  if (rules.length > 32 || new Set(rules.map((rule) => rule.id)).size !== rules.length) return false
  return rules.every((rule) => rule.id.length > 0 && utf8Length(rule.id) <= 128 && !/[\x00-\x1f\x7f]/.test(rule.id)
    && utf8Length(rule.prefix) <= 1024 && !/[\x00-\x1f\x7f]/.test(rule.prefix)
    && (rule.type === 'expire_after' ? Number.isSafeInteger(rule.durationSeconds) && rule.durationSeconds > 0 : Number.isInteger(rule.count) && rule.count > 0 && rule.count <= 4_294_967_295))
}

function lifecycleDuration(seconds: number): string {
  if (seconds % 86_400 === 0) return `${seconds / 86_400} 天`
  if (seconds % 3_600 === 0) return `${seconds / 3_600} 小时`
  if (seconds % 60 === 0) return `${seconds / 60} 分钟`
  return `${seconds} 秒`
}

function LifecycleRulesEditor({ rules, onChange }: { rules: LifecycleRule[]; onChange: (rules: LifecycleRule[]) => void }) {
  const update = (index: number, change: (rule: LifecycleRule) => LifecycleRule) => onChange(rules.map((rule, current) => current === index ? change(rule) : rule))
  const add = () => {
    if (rules.length >= 32) return
    onChange([...rules, { id: `rule-${crypto.randomUUID().slice(0, 8)}`, enabled: true, prefix: '', type: 'expire_after', durationSeconds: 604_800 }])
  }
  return <section className="space-y-4 border-t border-separator pt-5">
    <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
      <div><h3 className="text-sm font-semibold">自动清理</h3><p className="mt-1 max-w-2xl text-xs leading-5 text-muted">由后台任务根据对象创建时间和 Key 前缀执行，不会影响上传和校验。</p></div>
      <Button type="button" variant="secondary" size="sm" isDisabled={rules.length >= 32} onClick={add}><Plus className="size-4" />添加规则</Button>
    </div>
    <Alert status="accent"><Alert.Indicator><CircleHelp className="size-4" /></Alert.Indicator><Alert.Content><Alert.Title>什么时候需要它？</Alert.Title><Alert.Description>临时文件用“按时间删除”，连续导出或备份用“只保留最新”。普通永久素材可以不配置。</Alert.Description></Alert.Content></Alert>
    {rules.length === 0 ? <div className="rounded-lg border border-dashed border-separator px-5 py-8 text-center"><p className="text-sm font-medium">当前不会自动清理</p><p className="mt-1 text-xs text-muted">对象将按自身 TTL 或永久保留。</p></div> : <div className="space-y-3">{rules.map((rule, index) => <Card key={`${rule.id}-${index}`} variant="secondary">
      <Card.Header className="flex flex-col gap-3 border-b border-separator px-4 py-3 sm:flex-row sm:items-center">
        <Switch isSelected={rule.enabled} onChange={(enabled) => update(index, (current) => ({ ...current, enabled }))} aria-label={`启用规则 ${index + 1}`}><Switch.Control><Switch.Thumb /></Switch.Control><Switch.Content>{rule.enabled ? '已启用' : '已停用'}</Switch.Content></Switch>
        <SelectControl aria-label={`规则 ${index + 1} 类型`} className="min-w-44 sm:w-auto" value={rule.type} options={[{ value: 'expire_after', label: '按时间删除' }, { value: 'keep_latest', label: '只保留最新' }]} onChange={(next) => update(index, (current) => next === 'expire_after' ? { id: current.id, enabled: current.enabled, prefix: current.prefix, type: 'expire_after', durationSeconds: 604_800 } : { id: current.id, enabled: current.enabled, prefix: current.prefix, type: 'keep_latest', count: 1 })} />
        <Chip className="sm:ml-auto" size="sm" variant="soft"><Chip.Label>{rule.prefix ? `前缀 ${rule.prefix}` : '整个 Bucket'}</Chip.Label></Chip>
        <Button type="button" isIconOnly size="sm" variant="ghost" aria-label="删除规则" onClick={() => onChange(rules.filter((_, current) => current !== index))}><Trash2 className="size-4 text-danger" /></Button>
      </Card.Header>
      <Card.Content className="grid gap-4 px-4 py-4 sm:grid-cols-2">
        <label><span className="mb-1.5 block text-xs font-medium">Object Key 前缀</span><Input value={rule.prefix} placeholder="留空表示整个 Bucket" onChange={(event) => update(index, (current) => ({ ...current, prefix: event.target.value }))} /></label>
        {rule.type === 'expire_after' ? <label><span className="mb-1.5 flex justify-between text-xs font-medium"><span>保留秒数</span><span className="text-muted">{lifecycleDuration(rule.durationSeconds)}</span></span><Input type="number" min={1} value={String(rule.durationSeconds || '')} onChange={(event) => update(index, (current) => current.type === 'expire_after' ? { ...current, durationSeconds: Number(event.target.value) } : current)} /></label> : <label><span className="mb-1.5 block text-xs font-medium">保留最新数量</span><Input type="number" min={1} max={4_294_967_295} value={String(rule.count || '')} onChange={(event) => update(index, (current) => current.type === 'keep_latest' ? { ...current, count: Number(event.target.value) } : current)} /></label>}
        <details className="sm:col-span-2"><summary className="cursor-pointer text-xs font-medium text-muted">高级设置</summary><label className="mt-3 block"><span className="mb-1.5 block text-xs font-medium">规则 ID</span><Input className="font-mono text-xs" maxLength={128} value={rule.id} onChange={(event) => update(index, (current) => ({ ...current, id: event.target.value }))} /></label></details>
      </Card.Content>
    </Card>)}</div>}
  </section>
}

function BucketEditor({ bucket, pending, error, onClose, onSave }: { bucket: Bucket | null; pending: boolean; error: unknown; onClose: () => void; onSave: (input: BucketInput) => void }) {
  const [name, setName] = useState(bucket?.name ?? '')
  const [visibility, setVisibility] = useState<'public' | 'private'>(bucket?.visibility === '公开' ? 'public' : 'private')
  const [ttl, setTtl] = useState(bucket?.defaultTtlSeconds?.toString() ?? '')
  const [maxSize, setMaxSize] = useState(bucket?.maxObjectSize?.toString() ?? '')
  const [mimes, setMimes] = useState(bucket?.allowedMimeTypes.join(', ') ?? '')
  const [rules, setRules] = useState<LifecycleRule[]>(() => structuredClone(bucket?.lifecycleRules ?? []))
  const rulesValid = lifecycleRulesValid(rules)
  const submit = (event: React.FormEvent) => {
    event.preventDefault()
    if (!rulesValid) return
    onSave({ name: name.trim(), visibility, defaultTtlSeconds: ttl ? Number(ttl) : null, maxObjectSize: maxSize ? Number(maxSize) : null, allowedMimeTypes: mimes.split(',').map((value) => value.trim()).filter(Boolean), lifecycleRules: structuredClone(rules) })
  }
  return <Modal title={bucket ? '编辑 Bucket' : '新建 Bucket'} onClose={onClose} wide><form onSubmit={submit} className="space-y-5">
    <section><div className="mb-3"><h3 className="text-sm font-semibold">基础设置</h3><p className="mt-1 text-xs text-muted">名称创建后不可修改；可见性将作为对象默认访问范围。</p></div><div className="grid gap-4 sm:grid-cols-2"><label><span className="mb-1.5 block text-xs font-medium text-muted">名称</span><Input fullWidth required disabled={Boolean(bucket)} value={name} onChange={(event) => setName(event.target.value)} /></label><label><span className="mb-1.5 block text-xs font-medium text-muted">可见性</span><SelectControl aria-label="Bucket 可见性" value={visibility} options={[{ value: 'private', label: '私有' }, { value: 'public', label: '公开' }]} onChange={(next) => setVisibility(next as 'public' | 'private')} /></label></div></section>
    <section className="border-t border-separator pt-5"><div className="mb-3"><h3 className="text-sm font-semibold">对象约束</h3><p className="mt-1 text-xs text-muted">留空表示不设置默认过期时间、大小或 MIME 限制。</p></div><div className="grid gap-4 sm:grid-cols-2"><label><span className="mb-1.5 block text-xs font-medium text-muted">默认 TTL（秒）</span><Input fullWidth type="number" min={1} placeholder="永久" value={ttl} onChange={(event) => setTtl(event.target.value)} /></label><label><span className="mb-1.5 block text-xs font-medium text-muted">单对象上限（字节）</span><Input fullWidth type="number" min={1} placeholder="不限" value={maxSize} onChange={(event) => setMaxSize(event.target.value)} /></label><label className="sm:col-span-2"><span className="mb-1.5 block text-xs font-medium text-muted">允许的 MIME（逗号分隔）</span><Input fullWidth placeholder="image/png, image/jpeg" value={mimes} onChange={(event) => setMimes(event.target.value)} /></label></div></section>
    <LifecycleRulesEditor rules={rules} onChange={setRules} />
    <MutationError error={error} /><ModalActions onCancel={onClose} pending={pending} submitLabel={bucket ? '保存策略' : '创建 Bucket'} disabled={!name.trim() || !rulesValid} />
  </form></Modal>
}

function AccessKeysPage() {
  const { appId = '' } = useParams()
  const queryClient = useQueryClient()
  const keys = useQuery({ queryKey: ['access-keys', appId], queryFn: () => api.getAccessKeys(appId) })
  const [editor, setEditor] = useState<AccessKey | null | undefined>(undefined)
  const [secret, setSecret] = useState<OneTimeSecret | null>(null)
  const refresh = () => queryClient.invalidateQueries({ queryKey: ['access-keys', appId] })
  const save = useMutation<OneTimeSecret | AccessKey, Error, { name: string; permissions: Permission[]; expiresAt: string | null }>({
    mutationFn: (input: { name: string; permissions: Permission[]; expiresAt: string | null }) => editor ? api.updateAccessKey(editor.id, input) : api.createAccessKey(appId, input),
    onSuccess: async (result) => { if ('secret' in result) setSecret(result); await refresh(); setEditor(undefined) },
  })
  const rotate = useMutation({ mutationFn: (key: AccessKey) => api.rotateAccessKey(appId, key), onSuccess: async (value) => { setSecret(value); await refresh() } })
  const revoke = useMutation({ mutationFn: api.revokeAccessKey, onSuccess: refresh })
  return <>
    <MutationError error={keys.error ?? rotate.error ?? revoke.error} />
    {secret && <OneTimeSecretPanel value={secret} onClose={() => setSecret(null)} />}
    <Card variant="default" className="overflow-hidden">
      <Card.Header className="flex flex-col gap-3 border-b border-separator px-5 py-4 sm:flex-row sm:items-center sm:justify-between"><div><h1 className="text-base font-semibold">访问密钥</h1><p className="mt-1 text-xs text-muted">为服务创建最小权限凭证，按职责授予必要权限。</p></div><div className="flex items-center gap-2"><Chip size="sm" variant="soft"><Chip.Label>{keys.data?.filter((key) => key.status === '有效').length ?? 0} 个有效</Chip.Label></Chip><Button variant="primary" size="sm" onClick={() => setEditor(null)}><Plus className="size-4" />创建访问密钥</Button></div></Card.Header>
      <Card.Content className="p-0">{keys.isLoading ? <PageLoading /> : keys.data?.length ? <div className="overflow-x-auto"><table className="w-full min-w-[840px]">
        <thead className="bg-[#f7f9fc] text-left text-[11px] font-semibold text-muted"><tr><th className="px-5 py-3">名称与标识</th><th className="px-5 py-3">权限</th><th className="px-5 py-3">上次使用</th><th className="px-5 py-3">过期时间</th><th className="px-5 py-3">状态</th><th aria-label="操作" className="w-32 px-3 py-3" /></tr></thead>
        <tbody className="divide-y divide-separator">{keys.data.map((key) => <tr className="text-sm transition-colors hover:bg-[#f8fbff]" key={key.id}>
          <td className="px-5 py-3.5"><div className="flex min-w-0 items-center gap-3"><span className="grid size-8 shrink-0 place-items-center rounded-md bg-[#eefaf8] text-[#0f766e]"><KeyRound className="size-4" /></span><div className="min-w-0"><span className="block truncate font-medium">{key.name}</span><code className="mt-0.5 block max-w-56 truncate text-[11px] text-muted" title={key.id}>{key.id}</code></div></div></td>
          <td className="max-w-80 px-5 py-3.5"><div className="flex flex-wrap gap-1" title={key.scope}>{key.permissions.slice(0, 2).map((permission) => <Chip size="sm" variant="soft" key={permission}><Chip.Label><code className="text-[10px]">{permission}</code></Chip.Label></Chip>)}{key.permissions.length > 2 && <Chip size="sm" variant="soft"><Chip.Label>+{key.permissions.length - 2}</Chip.Label></Chip>}</div></td>
          <td className="px-5 py-3.5 text-xs text-muted">{key.lastUsed}</td><td className="px-5 py-3.5 text-xs text-muted">{key.expiresAt ? formatDateTime(key.expiresAt) : '不过期'}</td><td className="px-5 py-3.5"><Badge tone={key.status === '有效' ? 'positive' : 'danger'}>{key.status}</Badge></td>
          <td className="px-3 py-3.5"><div className="flex justify-end gap-1">{key.status === '有效' && <><Button isIconOnly size="sm" variant="ghost" aria-label="编辑访问密钥" onClick={() => setEditor(key)}><Pencil className="size-4" /></Button><Button isIconOnly size="sm" variant="ghost" aria-label="轮换访问密钥" isDisabled={rotate.isPending} onClick={() => { if (window.confirm(`创建新密钥并撤销 “${key.name}”？`)) rotate.mutate(key) }}><RefreshCw className="size-4" /></Button><Button isIconOnly size="sm" variant="ghost" className="text-danger" aria-label="撤销访问密钥" isDisabled={revoke.isPending} onClick={() => { if (window.confirm(`撤销 “${key.name}”？`)) revoke.mutate(key.id) }}><Trash2 className="size-4" /></Button></>}</div></td>
        </tr>)}</tbody>
      </table></div> : <EmptyState icon={KeyRound} title="还没有访问密钥" description="创建最小权限密钥供程序调用。" action={<Button variant="primary" onClick={() => setEditor(null)}><Plus className="size-4" />创建访问密钥</Button>} />}<div className="flex items-start gap-3 border-t border-separator bg-accent-soft/35 px-5 py-3 text-xs leading-5 text-muted"><ShieldCheck className="mt-0.5 size-4 shrink-0 text-accent" /><p><span className="font-medium text-foreground">Secret 仅显示一次。</span> 控制台不会保存或再次读取 SecretAccessKey，关闭后无法恢复。</p></div></Card.Content>
    </Card>
    {editor !== undefined && <AccessKeyEditor accessKey={editor} pending={save.isPending} error={save.error} onClose={() => setEditor(undefined)} onSave={(input) => save.mutate(input)} />}
  </>
}

const accessKeyPermissions: Permission[] = ['application:read', 'bucket:list', 'bucket:manage', 'media:list', 'media:read', 'media:upload', 'media:update', 'media:delete', 'webhook:manage']
export function AccessKeyEditor({ accessKey, pending, error, onClose, onSave }: { accessKey: AccessKey | null; pending: boolean; error: unknown; onClose: () => void; onSave: (input: { name: string; permissions: Permission[]; expiresAt: string | null }) => void }) {
  const [name, setName] = useState(accessKey?.name ?? '')
  const [permissions, setPermissions] = useState<Permission[]>(accessKey?.permissions ?? ['media:read'])
  const [expiresAt, setExpiresAt] = useState(accessKey?.expiresAt?.slice(0, 16) ?? '')
  const toggle = (permission: Permission) => setPermissions((current) => current.includes(permission) ? current.filter((value) => value !== permission) : [...current, permission])
  return <Modal title={accessKey ? '编辑访问密钥' : '创建访问密钥'} onClose={onClose}><form className="space-y-5" onSubmit={(event) => { event.preventDefault(); onSave({ name: name.trim(), permissions, expiresAt: expiresAt ? new Date(expiresAt).toISOString() : null }) }}>
    <label><span className="mb-1.5 block text-xs font-medium text-muted">名称</span><Input fullWidth required maxLength={128} placeholder="例如：生产环境上传服务" value={name} onChange={(event) => setName(event.target.value)} /></label>
    <fieldset><div className="mb-3 flex items-end justify-between gap-3"><div><legend className="text-sm font-semibold">权限</legend><p className="mt-1 text-xs text-muted">仅启用该服务实际需要的能力。</p></div><span className="text-xs tabular-nums text-muted">已选 {permissions.length}</span></div><div className="grid gap-2 sm:grid-cols-2">{accessKeyPermissions.map((permission) => <Checkbox className="min-h-11 rounded-md border border-separator bg-white px-3 py-2" isSelected={permissions.includes(permission)} onChange={() => toggle(permission)} key={permission}><Checkbox.Content className="flex w-full cursor-pointer items-center justify-between gap-3" aria-label={permission}><code className="text-xs">{permission}</code><Checkbox.Control><Checkbox.Indicator /></Checkbox.Control></Checkbox.Content></Checkbox>)}</div></fieldset>
    <label><span className="mb-1.5 block text-xs font-medium text-muted">过期时间</span><Input fullWidth type="datetime-local" value={expiresAt} onChange={(event) => setExpiresAt(event.target.value)} /><span className="mt-1.5 block text-xs text-muted">留空表示长期有效，可随时从列表撤销。</span></label>
    <MutationError error={error} /><ModalActions onCancel={onClose} pending={pending} submitLabel={accessKey ? '保存密钥' : '创建密钥'} disabled={!name.trim() || permissions.length === 0} />
  </form></Modal>
}

function WebhooksPage() {
  const { appId = '' } = useParams()
  const queryClient = useQueryClient()
  const hooks = useQuery({ queryKey: ['webhooks', appId], queryFn: api.getWebhooks })
  const [editor, setEditor] = useState<WebhookEndpoint | null | undefined>(undefined)
  const [deliveryEndpoint, setDeliveryEndpoint] = useState<WebhookEndpoint | null>(null)
  const [secret, setSecret] = useState<OneTimeSecret | null>(null)
  const refresh = () => queryClient.invalidateQueries({ queryKey: ['webhooks', appId] })
  const save = useMutation<OneTimeSecret | WebhookEndpoint, Error, WebhookInput>({ mutationFn: (input) => editor ? api.updateWebhook(editor.id, input) : api.createWebhook(input), onSuccess: async (value) => { if ('secret' in value) setSecret(value); await refresh(); setEditor(undefined) } })
  const rotate = useMutation({ mutationFn: api.rotateWebhookSecret, onSuccess: async (value) => { setSecret(value); await refresh() } })
  const remove = useMutation({ mutationFn: api.deleteWebhook, onSuccess: refresh })
  return <>
    <MutationError error={hooks.error ?? rotate.error ?? remove.error} />
    {secret && <OneTimeSecretPanel value={secret} onClose={() => setSecret(null)} />}
    <Card variant="default" className="overflow-hidden"><Card.Header className="flex flex-col gap-3 border-b border-separator px-5 py-4 sm:flex-row sm:items-center sm:justify-between"><div><h1 className="text-base font-semibold">Webhook</h1><p className="mt-1 text-xs text-muted">配置事件订阅端点，查看运行状态和最近投递。</p></div><div className="flex items-center gap-2"><Chip size="sm" variant="soft"><Chip.Label>{hooks.data?.filter((hook) => hook.enabled).length ?? 0} 个启用</Chip.Label></Chip><Button variant="primary" size="sm" onClick={() => setEditor(null)}><Plus className="size-4" />添加端点</Button></div></Card.Header><Card.Content className="p-0">
      {hooks.isLoading ? <PageLoading /> : hooks.data?.length ? <div className="overflow-x-auto"><table className="w-full min-w-[820px]">
        <thead className="bg-[#f7f9fc] text-left text-[11px] font-semibold text-muted"><tr><th className="px-5 py-3">端点</th><th className="px-5 py-3">订阅事件</th><th className="px-5 py-3">状态</th><th className="px-5 py-3">最近投递</th><th aria-label="操作" className="w-40 px-3 py-3" /></tr></thead>
        <tbody className="divide-y divide-separator">{hooks.data.map((hook) => <tr className="text-sm transition-colors hover:bg-[#f8fbff]" key={hook.id}>
          <td className="px-5 py-3.5"><div className="min-w-0"><span className="block max-w-80 truncate font-medium" title={hook.url}>{hook.url}</span><code className="mt-0.5 block max-w-64 truncate text-[11px] text-muted" title={hook.id}>{hook.id}</code></div></td>
          <td className="max-w-96 px-5 py-3.5"><div className="flex flex-wrap gap-1">{hook.events.slice(0, 2).map((event) => <Chip size="sm" variant="soft" key={event}><Chip.Label><code className="text-[10px]">{event}</code></Chip.Label></Chip>)}{hook.events.length > 2 && <Chip size="sm" variant="soft"><Chip.Label>+{hook.events.length - 2}</Chip.Label></Chip>}</div></td>
          <td className="px-5 py-3.5"><Badge tone={hook.state === '健康' ? 'positive' : 'neutral'}>{hook.state}</Badge></td><td className="px-5 py-3.5 text-xs text-muted">{hook.latest}</td>
          <td className="px-3 py-3.5"><div className="flex justify-end gap-1"><Button isIconOnly size="sm" variant="ghost" aria-label="查看投递历史" onClick={() => setDeliveryEndpoint(hook)}><Eye className="size-4" /></Button><Button isIconOnly size="sm" variant="ghost" aria-label="编辑 Webhook" onClick={() => setEditor(hook)}><Pencil className="size-4" /></Button><Button isIconOnly size="sm" variant="ghost" aria-label="轮换 Secret" isDisabled={rotate.isPending} onClick={() => { if (window.confirm('轮换这个 Webhook 的 Secret？')) rotate.mutate(hook.id) }}><RefreshCw className="size-4" /></Button><Button isIconOnly size="sm" variant="ghost" className="text-danger" aria-label="删除 Webhook" isDisabled={remove.isPending} onClick={() => { if (window.confirm('删除这个 Webhook？')) remove.mutate(hook.id) }}><Trash2 className="size-4" /></Button></div></td>
        </tr>)}</tbody>
      </table></div> : <EmptyState icon={Webhook} title="还没有 Webhook" description="添加 HTTPS 端点订阅媒体事件。" action={<Button variant="primary" onClick={() => setEditor(null)}><Plus className="size-4" />添加端点</Button>} />}
    </Card.Content></Card>
    {editor !== undefined && <WebhookEditor endpoint={editor} pending={save.isPending} error={save.error} onClose={() => setEditor(undefined)} onSave={(input) => save.mutate(input)} />}{deliveryEndpoint && <WebhookDeliveriesModal endpoint={deliveryEndpoint} onClose={() => setDeliveryEndpoint(null)} />}
  </>
}

const webhookEvents = ['media.uploaded', 'media.metadata_updated', 'media.delete_scheduled', 'media.deleted']
function WebhookEditor({ endpoint, pending, error, onClose, onSave }: { endpoint: WebhookEndpoint | null; pending: boolean; error: unknown; onClose: () => void; onSave: (input: WebhookInput) => void }) {
  const [url, setUrl] = useState(endpoint?.url ?? '')
  const [events, setEvents] = useState(endpoint?.events ?? ['media.uploaded'])
  const [enabled, setEnabled] = useState(endpoint?.enabled ?? true)
  const toggle = (value: string) => setEvents((current) => current.includes(value) ? current.filter((event) => event !== value) : [...current, value])
  return <Modal title={endpoint ? '编辑 Webhook' : '添加 Webhook'} onClose={onClose}><form className="space-y-5" onSubmit={(event) => { event.preventDefault(); onSave({ url: url.trim(), events, enabled }) }}>
    <label><span className="mb-1.5 block text-xs font-medium text-muted">HTTPS 端点</span><Input fullWidth required type="url" placeholder="https://example.com/hooks/media" value={url} onChange={(event) => setUrl(event.target.value)} /></label>
    <fieldset><div className="mb-3"><legend className="text-sm font-semibold">订阅事件</legend><p className="mt-1 text-xs text-muted">仅发送已选事件；至少选择一项。</p></div><div className="grid gap-2">{webhookEvents.map((value) => <div className="flex min-h-11 items-center justify-between gap-3 rounded-md border border-separator bg-white px-3 py-2" key={value}><code className="text-xs">{value}</code><Switch isSelected={events.includes(value)} aria-label={`订阅事件 ${value}`} onChange={() => toggle(value)}><Switch.Control><Switch.Thumb /></Switch.Control></Switch></div>)}</div></fieldset>
    <div className="flex items-center justify-between gap-4 rounded-md border border-separator bg-[#f7f9fc] px-4 py-3"><div><p className="text-sm font-medium">启用端点</p><p className="mt-0.5 text-xs text-muted">停用后保留配置，但不再创建新投递。</p></div><Switch isSelected={enabled} aria-label="启用端点" onChange={setEnabled}><Switch.Control><Switch.Thumb /></Switch.Control></Switch></div>
    <MutationError error={error} /><ModalActions onCancel={onClose} pending={pending} submitLabel={endpoint ? '保存 Webhook' : '添加 Webhook'} disabled={!url.trim() || events.length === 0} />
  </form></Modal>
}

function WebhookDeliveriesModal({ endpoint, onClose }: { endpoint: WebhookEndpoint; onClose: () => void }) {
  const queryClient = useQueryClient()
  const [status, setStatus] = useState<WebhookDeliveryStatus | ''>('')
  const deliveries = useInfiniteQuery({
    queryKey: ['webhook-deliveries', endpoint.id, status],
    initialPageParam: '',
    queryFn: ({ pageParam }) => api.getWebhookDeliveries(endpoint.id, { status: status || undefined, limit: 20, cursor: pageParam || undefined }),
    getNextPageParam: (page) => page.nextCursor ?? undefined,
  })
  const replay = useMutation({ mutationFn: (eventId: string) => api.replayWebhookDelivery(endpoint.id, eventId), onSuccess: () => queryClient.invalidateQueries({ queryKey: ['webhook-deliveries', endpoint.id] }) })
  const items = deliveries.data?.pages.flatMap((page) => page.items) ?? []
  return <Modal title="Webhook 投递历史" onClose={onClose} wide>
    <div className="mb-4 flex flex-col gap-3 rounded-md border border-separator bg-[#f7f9fc] px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
      <div className="min-w-0"><p className="truncate text-sm font-medium" title={endpoint.url}>{endpoint.url}</p><code className="mt-1 block truncate text-[11px] text-muted" title={endpoint.id}>{endpoint.id}</code></div>
      <div className="flex shrink-0 gap-2"><SelectControl aria-label="投递状态" className="min-w-40" value={status} options={[{ value: '', label: '全部状态' }, { value: 'pending', label: 'Pending' }, { value: 'delivered', label: 'Delivered' }, { value: 'dead_lettered', label: 'Dead lettered' }]} onChange={(next) => setStatus(next as WebhookDeliveryStatus | '')} /><Button isIconOnly variant="ghost" aria-label="刷新投递历史" onClick={() => deliveries.refetch()}><RefreshCw className={cn('size-4', deliveries.isFetching && 'animate-spin')} /></Button></div>
    </div>
    <MutationError error={deliveries.error ?? replay.error} />
    {deliveries.isLoading ? <PageLoading /> : items.length ? <>
      <div className="overflow-x-auto rounded-md border border-separator"><table className="w-full min-w-[900px]"><thead className="bg-[#f7f9fc] text-left text-[11px] font-semibold text-muted"><tr><th className="px-4 py-3">事件</th><th className="px-4 py-3">状态</th><th className="px-4 py-3">尝试</th><th className="px-4 py-3">HTTP</th><th className="px-4 py-3">最近更新</th><th className="px-4 py-3">错误</th><th aria-label="操作" className="w-24 px-3 py-3" /></tr></thead><tbody className="divide-y divide-separator">{items.map((delivery) => <WebhookDeliveryRow key={`${delivery.eventId}-${delivery.updatedAt}`} delivery={delivery} replaying={replay.isPending} onReplay={() => replay.mutate(delivery.eventId)} />)}</tbody></table></div>
      {deliveries.hasNextPage && <div className="mt-4 flex justify-center"><Button variant="secondary" isDisabled={deliveries.isFetchingNextPage} onClick={() => deliveries.fetchNextPage()}>{deliveries.isFetchingNextPage && <LoaderCircle className="size-4 animate-spin" />}加载更多</Button></div>}
    </> : <EmptyState icon={Webhook} title="没有投递记录" description="当前端点与状态过滤下没有历史记录。" />}
  </Modal>
}

function WebhookDeliveryRow({ delivery, replaying, onReplay }: { delivery: WebhookDelivery; replaying: boolean; onReplay: () => void }) {
  const tone = delivery.status === 'delivered' ? 'positive' : delivery.status === 'dead_lettered' ? 'danger' : 'warning'
  const replayable = delivery.status === 'delivered' || delivery.status === 'dead_lettered'
  return <tr className="text-xs transition-colors hover:bg-[#f8fbff]"><td className="px-4 py-3"><span className="block font-medium">{delivery.eventType}</span><code className="mt-1 block max-w-52 truncate text-[10px] text-muted" title={delivery.eventId}>{delivery.eventId}</code></td><td className="px-4 py-3"><Badge tone={tone}>{delivery.status}</Badge></td><td className="px-4 py-3 tabular-nums text-muted"><span className="block">{delivery.attemptCount}</span>{delivery.replayCount > 0 && <span className="mt-1 block text-[10px]">重放 {delivery.replayCount}</span>}</td><td className="px-4 py-3">{delivery.lastResponseStatus ? <code className={delivery.lastResponseStatus >= 400 ? 'text-danger' : 'text-success'}>{delivery.lastResponseStatus}</code> : <span className="text-muted">--</span>}</td><td className="px-4 py-3 text-muted">{formatDateTime(delivery.updatedAt)}</td><td className="max-w-64 px-4 py-3"><span className={cn('block truncate', delivery.lastError ? 'text-danger' : 'text-muted')} title={delivery.lastError ?? undefined}>{delivery.lastError ?? '--'}</span></td><td className="px-3 py-3"><Button variant="ghost" size="sm" className="text-xs" isDisabled={!replayable || replaying} aria-label={replayable ? '手动重放' : '投递完成或进入死信后可重放'} onClick={onReplay}><RefreshCw className={cn('size-3.5', replaying && 'animate-spin')} />重放</Button></td></tr>
}

function SettingsPage() {
  const { appId = '' } = useParams()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const applications = useQuery({ queryKey: ['applications'], queryFn: api.getApplications })
  const app = applications.data?.find((item) => item.appId === appId)
  const [name, setName] = useState('')
  useEffect(() => { if (app) setName(app.name) }, [app])
  const update = useMutation({ mutationFn: () => api.updateApplication(appId, name.trim()), onSuccess: () => queryClient.invalidateQueries({ queryKey: ['applications'] }) })
  const remove = useMutation({ mutationFn: () => api.deleteApplication(appId), onSuccess: async () => { await queryClient.invalidateQueries({ queryKey: ['applications'] }); navigate('/') } })
  if (applications.isLoading) return <PageLoading />
  return <>
    <MutationError error={applications.error ?? update.error ?? remove.error} />
    <div className="grid items-start gap-4 xl:grid-cols-[minmax(0,1.2fr)_minmax(360px,.8fr)]">
      <Card variant="default">
        <Card.Header className="border-b border-separator px-5 py-4"><h1 className="text-base font-semibold">设置</h1><p className="mt-1 text-xs text-muted">管理应用身份与资源归属；名称调整不会影响对象访问路径。</p></Card.Header>
        <Card.Content className="px-5 py-5">
          <form onSubmit={(event) => { event.preventDefault(); if (name.trim()) update.mutate() }}>
            <div className="grid gap-5 lg:grid-cols-2">
              <label><span className="mb-2 block text-xs font-medium text-muted">应用名称</span><Input fullWidth maxLength={128} value={name} onChange={(event) => setName(event.target.value)} /></label>
              <label><span className="mb-2 block text-xs font-medium text-muted">AppId</span><div className="flex gap-2"><Input fullWidth className="font-mono text-xs" readOnly value={app?.appId ?? appId} /><Button isIconOnly variant="secondary" type="button" aria-label="复制 AppId" onClick={() => void navigator.clipboard.writeText(appId)}><Copy className="h-4 w-4" /></Button></div></label>
            </div>
            <div className="mt-6 flex justify-end"><Button type="submit" variant="primary" isDisabled={update.isPending || !name.trim()}>{update.isPending && <LoaderCircle className="h-4 w-4 animate-spin" />}保存更改</Button></div>
          </form>
        </Card.Content>
      </Card>

      <div>
        <Card variant="default" className="border-danger/25">
          <Card.Header className="border-b border-danger/15 px-5 py-4"><Card.Title className="text-sm font-semibold text-danger">危险操作</Card.Title><Card.Description className="mt-1 text-xs">仅空应用可以被永久删除。</Card.Description></Card.Header>
          <Card.Content className="flex flex-col gap-4 px-5 py-4 sm:flex-row sm:items-center sm:justify-between"><p className="text-xs leading-5 text-muted">删除后 AppId 无法恢复，相关审计记录仍会保留。</p><Button variant="danger-soft" className="shrink-0" isDisabled={remove.isPending} onClick={() => { if (window.confirm(`永久删除应用 “${app?.name ?? appId}”？`)) remove.mutate() }}><Trash2 className="h-4 w-4" />删除应用</Button></Card.Content>
        </Card>
      </div>

      <div className="xl:col-span-2"><SessionsPanel /></div>
    </div>
  </>
}

function SessionsPanel() {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const sessions = useQuery({ queryKey: ['auth', 'sessions'], queryFn: api.getSessions })
  const leave = () => { queryClient.setQueryData(['auth', 'me'], null); queryClient.removeQueries({ queryKey: ['auth', 'sessions'] }); navigate('/login', { replace: true }) }
  const revoke = useMutation<void, Error, AuthSession>({
    mutationFn: (session) => api.revokeSession(session.id),
    onSuccess: async (_, session) => { if (session.isCurrent) leave(); else await queryClient.invalidateQueries({ queryKey: ['auth', 'sessions'] }) },
  })
  const revokeAll = useMutation({ mutationFn: api.revokeAllSessions, onSuccess: leave })
  return <Card variant="default" className="overflow-hidden"><Card.Header className="flex flex-col gap-3 border-b border-separator px-5 py-4 sm:flex-row sm:items-center sm:justify-between"><div><Card.Title className="text-sm font-semibold">活跃 Session</Card.Title><Card.Description className="mt-1 text-xs">最近登录的浏览器、网络位置和到期时间。</Card.Description></div><Button variant="danger-soft" size="sm" isDisabled={revokeAll.isPending || !sessions.data?.length} onClick={() => { if (window.confirm('撤销全部 Session 并退出登录？')) revokeAll.mutate() }}><LogOut className="h-4 w-4" />撤销全部</Button></Card.Header><Card.Content className="p-0"><MutationError error={sessions.error ?? revoke.error ?? revokeAll.error} />{sessions.isLoading ? <div className="grid min-h-32 place-items-center"><Spinner aria-label="加载 Session" color="accent" /></div> : sessions.data?.length ? <div className="divide-y divide-separator">{sessions.data.map((session) => <div className="grid grid-cols-[auto_minmax(0,1fr)_auto] items-start gap-3 px-5 py-4" key={session.id}><span className="grid h-9 w-9 place-items-center rounded-md bg-[#ecfdf5] text-[#0f766e]"><Monitor className="h-4 w-4" /></span><div className="min-w-0"><div className="flex min-w-0 flex-wrap items-center gap-2"><p className="max-w-3xl truncate text-sm font-medium">{session.userAgent ?? '未知浏览器'}</p>{session.isCurrent && <Badge tone="positive">当前 Session</Badge>}</div><p className="mt-1 text-xs text-muted">{session.lastSeenIp ?? session.createdIp ?? '未知 IP'} · 最近活动 {formatDateTime(session.lastSeenAt)}</p><p className="mt-0.5 truncate font-mono text-[10px] text-muted" title={session.id}>{session.id} · 到期 {formatDateTime(session.expiresAt)}</p></div><Button variant="ghost" size="sm" className="text-danger" isDisabled={revoke.isPending} onClick={() => { if (window.confirm(session.isCurrent ? '撤销当前 Session 并退出登录？' : '撤销这个 Session？')) revoke.mutate(session) }}>{session.isCurrent ? '退出' : '撤销'}</Button></div>)}</div> : <p className="py-10 text-center text-sm text-muted">没有活跃 Session</p>}</Card.Content></Card>
}

function formatDateTime(value: string): string {
  const normalized = value.replace(/^(\d{4}-\d{2}-\d{2})[ T](\d{2}:\d{2}:\d{2}(?:\.\d+)?) ([+-]\d{2}:\d{2}):\d{2}$/, '$1T$2$3')
  const date = new Date(normalized)
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString()
}

function ObjectDetailPage() {
  const { appId = '', mediaId = '' } = useParams()
  const navigate = useNavigate()
  const location = useLocation()
  const queryClient = useQueryClient()
  const objectQuery = useQuery({ queryKey: ['objects', appId, 'detail', mediaId], queryFn: () => api.getObject(mediaId) })
  const object = objectQuery.data
  const navigationState = getObjectListNavigationState(location.state)
  const returnTo = objectListReturnPath(appId, navigationState?.from)
  const returnState = navigationState ? { from: returnTo, objectList: navigationState.objectList } : undefined
  const [displayName, setDisplayName] = useState('')
  const [visibility, setVisibility] = useState<'' | 'public' | 'private'>('')
  const [ttl, setTtl] = useState('')
  const [clearTtl, setClearTtl] = useState(false)
  const [metadata, setMetadata] = useState('')
  const [metadataError, setMetadataError] = useState<string | null>(null)
  const [previewOpen, setPreviewOpen] = useState(false)
  const [variantPreviewOpen, setVariantPreviewOpen] = useState(false)
  const [deleteOpen, setDeleteOpen] = useState(false)
  const [variant, setVariant] = useState<VariantParams>(() => ({ ...DEFAULT_VARIANT_PARAMS }))
  useEffect(() => { if (object) setDisplayName(object.name) }, [object])
  const update = useMutation({
    mutationFn: async () => {
      if (!object) return
      let parsed: unknown
      if (metadata.trim()) {
        try { parsed = JSON.parse(metadata) } catch { throw new Error('Metadata 必须是合法 JSON') }
        if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed) || Object.keys(parsed as object).some((key) => key !== 'user' && key !== 'ai')) throw new Error('Metadata 只允许 user 和 ai 命名空间')
      }
      await api.updateObject(object.id, object.revision, { displayName: displayName.trim(), visibility: visibility || undefined, ttlSeconds: clearTtl ? null : ttl ? Number(ttl) : undefined, metadata: parsed as { user?: Record<string, unknown>; ai?: Record<string, unknown> } | undefined })
    },
    onSuccess: async () => { setMetadataError(null); await queryClient.invalidateQueries({ queryKey: ['objects', appId] }) },
    onError: (error) => setMetadataError(errorMessage(error)),
  })
  const remove = useMutation({ mutationFn: () => api.deleteObject(mediaId), onSuccess: async () => { await queryClient.invalidateQueries({ queryKey: ['objects', appId] }); navigate(returnTo, { state: returnState }) } })
  if (objectQuery.isLoading) return <PageLoading />
  if (objectQuery.error) return <MutationError error={objectQuery.error} />
  if (!object) return <EmptyState icon={Search} title="对象不存在" description="它可能已被删除或不属于当前应用。" />
  return <>
    <section className="mb-4 flex flex-col gap-4 rounded-lg border border-separator bg-surface p-4 shadow-sm sm:flex-row sm:items-center sm:justify-between">
      <div className="flex min-w-0 items-center gap-3"><LinkButton variant="ghost" className="size-9 shrink-0 p-0" to={returnTo} state={returnState}><ChevronLeft className="size-4" /><span className="sr-only">返回对象列表</span></LinkButton><span className="grid size-10 shrink-0 place-items-center rounded-md bg-[#eff6ff] text-[#2563eb]"><FileImage className="size-5" /></span><div className="min-w-0"><h1 className="truncate text-base font-semibold text-foreground">{object.name}</h1><code className="mt-1 block truncate text-[11px] text-muted" title={object.key}>{object.bucket}/{object.key}</code></div></div>
      <div className="flex shrink-0 gap-2"><Button variant="primary" onClick={() => setPreviewOpen(true)}><Eye className="size-4" />预览</Button><Button variant="danger-soft" onClick={() => { remove.reset(); setDeleteOpen(true) }}><Trash2 className="size-4" />删除</Button></div>
    </section>
    <div className="grid items-start gap-4 xl:grid-cols-[minmax(0,1fr)_380px]">
      <Card variant="default"><Card.Header className="border-b border-separator px-5 py-4"><Card.Title className="text-sm font-semibold">对象设置</Card.Title><Card.Description className="mt-1 text-xs">显示名称、访问覆盖、TTL 与业务 Metadata</Card.Description></Card.Header><Card.Content className="px-5 py-5"><form className="space-y-4" onSubmit={(event) => { event.preventDefault(); update.mutate() }}><label><span className="mb-2 block text-xs font-medium text-muted">显示名称</span><Input fullWidth value={displayName} onChange={(event) => setDisplayName(event.target.value)} /></label><div className="grid gap-4 sm:grid-cols-2"><label><span className="mb-2 block text-xs font-medium text-muted">可见性覆盖</span><SelectControl aria-label="对象可见性覆盖" value={visibility} options={[{ value: '', label: '保持不变' }, { value: 'private', label: '私有' }, { value: 'public', label: '公开' }]} onChange={(next) => setVisibility(next as '' | 'public' | 'private')} /></label><label><span className="mb-2 block text-xs font-medium text-muted">TTL（秒）</span><Input fullWidth type="number" min="1" disabled={clearTtl} placeholder="保持不变" value={ttl} onChange={(event) => setTtl(event.target.value)} /></label></div><Switch isSelected={clearTtl} onChange={(selected) => { setClearTtl(selected); if (selected) setTtl('') }}><Switch.Control><Switch.Thumb /></Switch.Control><Switch.Content>清除对象 TTL</Switch.Content></Switch><label><span className="mb-2 block text-xs font-medium text-muted">user / ai JSON</span><TextArea fullWidth className="min-h-40 font-mono text-xs" spellCheck={false} placeholder={'{\n  "user": { "project": "launch" }\n}'} value={metadata} onChange={(event) => setMetadata(event.target.value)} /></label>{metadataError && <p className="text-sm text-danger">{metadataError}</p>}<div className="flex justify-end"><Button type="submit" variant="primary" isDisabled={update.isPending || !displayName.trim()}>{update.isPending && <LoaderCircle className="h-4 w-4 animate-spin" />}保存更改</Button></div></form></Card.Content></Card>
      <div className="space-y-4">
        <Card variant="default"><Card.Header className="border-b border-separator px-5 py-4"><Card.Title className="text-sm font-semibold">对象属性</Card.Title></Card.Header><Card.Content className="px-5 py-4"><dl className="space-y-3 text-sm"><Detail label="对象 ID" value={object.id} mono /><Detail label="内容类型" value={object.type} /><Detail label="大小" value={formatBytes(object.size)} /><Detail label="Bucket" value={object.bucket} /><Detail label="Revision" value={object.revision.toString()} /><Detail label="SHA-256" value={object.sha256} mono /></dl></Card.Content></Card>
        {isRasterImageMimeType(object.type) && <Card variant="default" className="overflow-hidden"><VariantControls value={variant} pending={false} onChange={setVariant} onPreview={() => setVariantPreviewOpen(true)} /></Card>}
      </div>
    </div>
    {previewOpen && <ObjectPreviewModal item={object} onClose={() => setPreviewOpen(false)} />}
    {variantPreviewOpen && <ObjectPreviewModal item={object} variant={variant} onClose={() => setVariantPreviewOpen(false)} />}
    {deleteOpen && <DeleteObjectsModal item={object} count={1} pending={remove.isPending} error={remove.error} onClose={() => setDeleteOpen(false)} onConfirm={() => remove.mutate()} />}
  </>
}

function VariantParameterFields({ value, onChange, compact = false }: { value: VariantParams; onChange: (value: VariantParams) => void; compact?: boolean }) {
  const update = <K extends keyof VariantParams>(key: K, next: VariantParams[K]) => onChange({ ...value, [key]: next })
  const fieldClassName = compact ? 'w-24 shrink-0' : undefined
  const smallFieldClassName = compact ? 'w-20 shrink-0' : undefined
  const sliderClassName = compact ? 'w-28 shrink-0 pb-1' : undefined
  const backgroundValid = /^[a-fA-F0-9]{6}$/.test(value.background)
  return <div className={compact ? 'flex min-w-max items-end gap-2' : 'grid gap-3 sm:grid-cols-2 xl:grid-cols-4'}>
    <label className={smallFieldClassName}><span className="mb-1 block text-[10px] text-muted">宽度</span><Input aria-label="Variant 宽度" fullWidth type="number" min="1" max="4096" value={value.width} onChange={(event) => update('width', Number(event.target.value))} /></label>
    <label className={smallFieldClassName}><span className="mb-1 block text-[10px] text-muted">高度</span><Input aria-label="Variant 高度" fullWidth type="number" min="1" max="4096" value={value.height} onChange={(event) => update('height', Number(event.target.value))} /></label>
    <label className={fieldClassName}><span className="mb-1 block text-[10px] text-muted">适配</span><SelectControl aria-label="Variant 适配" value={value.fit} options={[{ value: 'cover', label: 'Cover' }, { value: 'contain', label: 'Contain' }, { value: 'inside', label: 'Inside' }]} onChange={(next) => update('fit', next as VariantParams['fit'])} /></label>
    <label className={fieldClassName}><span className="mb-1 block text-[10px] text-muted">格式</span><SelectControl aria-label="Variant 格式" value={value.format} options={[{ value: 'webp', label: 'WebP' }, { value: 'jpeg', label: 'JPEG' }, { value: 'png', label: 'PNG' }]} onChange={(next) => update('format', next as VariantParams['format'])} /></label>
    <label className={sliderClassName}><span className="mb-2 flex justify-between text-[10px] text-muted"><span>质量</span><span>{value.quality}</span></span><Slider aria-label="Variant 质量" minValue={1} maxValue={100} value={value.quality} onChange={(next) => update('quality', Array.isArray(next) ? next[0] : next)}><Slider.Track><Slider.Fill /><Slider.Thumb /></Slider.Track></Slider></label>
    <label className={sliderClassName}><span className="mb-2 flex justify-between text-[10px] text-muted"><span>模糊</span><span>{value.blur}</span></span><Slider aria-label="Variant 模糊" minValue={0} maxValue={100} value={value.blur} onChange={(next) => update('blur', Array.isArray(next) ? next[0] : next)}><Slider.Track><Slider.Fill /><Slider.Thumb /></Slider.Track></Slider></label>
    <label className={fieldClassName}><span className="mb-1 block text-[10px] text-muted">裁剪锚点</span><SelectControl aria-label="Variant 裁剪锚点" value={value.crop} options={[{ value: 'center', label: 'Center' }, { value: 'top', label: 'Top' }, { value: 'bottom', label: 'Bottom' }, { value: 'left', label: 'Left' }, { value: 'right', label: 'Right' }]} onChange={(next) => update('crop', next as VariantParams['crop'])} /></label>
    <label className={compact ? 'w-32 shrink-0' : undefined}><span className="mb-1 block text-[10px] text-muted">背景</span><div className="flex gap-1.5"><input aria-label="Variant 背景色选择" className="h-10 w-10 shrink-0 rounded-md border border-field-border bg-field p-1" type="color" value={backgroundValid ? `#${value.background}` : '#ffffff'} onChange={(event) => update('background', event.target.value.slice(1))} /><Input aria-label="Variant 背景色" fullWidth className="min-w-0 font-mono text-xs" maxLength={6} value={value.background} onChange={(event) => update('background', event.target.value.replace(/^#/, ''))} /></div></label>
  </div>
}

function VariantControls({ value, pending, onChange, onPreview }: { value: VariantParams; pending: boolean; onChange: (value: VariantParams) => void; onPreview: () => void }) {
  const valid = isValidVariantParams(value)
  return <div className="bg-surface p-4"><div className="mb-4 flex items-center justify-between"><h2 className="text-sm font-semibold">图片 Variant</h2><code className="text-[10px] text-muted">{value.width}×{value.height} {value.format}</code></div><VariantParameterFields value={value} onChange={onChange} /><Button variant="primary" className="mt-4" isDisabled={pending || !valid} onClick={onPreview}>{pending ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <Eye className="h-4 w-4" />}预览 Variant</Button></div>
}

function Modal({ title, onClose, children, wide = false, size, bodyClassName, containerClassName, dialogClassName, dismissable = true, showClose = true }: { title: string; onClose: () => void; children: React.ReactNode; wide?: boolean; size?: 'xs' | 'sm' | 'md' | 'lg' | 'full' | 'cover'; bodyClassName?: string; containerClassName?: string; dialogClassName?: string; dismissable?: boolean; showClose?: boolean }) {
  return <HeroModal isOpen onOpenChange={(open) => { if (!open && dismissable) onClose() }}>
    <HeroModal.Backdrop isDismissable={dismissable} variant="blur">
      <HeroModal.Container className={containerClassName} placement="center" scroll="inside" size={size ?? (wide ? 'lg' : 'md')}>
        <HeroModal.Dialog className={dialogClassName} aria-label={title}>
          <HeroModal.Header><HeroModal.Heading>{title}</HeroModal.Heading>{showClose && <HeroModal.CloseTrigger />}</HeroModal.Header>
          <HeroModal.Body className={bodyClassName}>{children}</HeroModal.Body>
        </HeroModal.Dialog>
      </HeroModal.Container>
    </HeroModal.Backdrop>
  </HeroModal>
}

function ModalActions({ onCancel, pending, submitLabel, disabled }: { onCancel: () => void; pending: boolean; submitLabel: string; disabled?: boolean }) {
  return <div className="flex justify-end gap-2 border-t border-separator pt-4"><Button variant="secondary" type="button" onClick={onCancel}>取消</Button><Button type="submit" variant="primary" isDisabled={pending || disabled}>{pending && <LoaderCircle className="h-4 w-4 animate-spin" />}{submitLabel}</Button></div>
}

function MutationError({ error }: { error: unknown }) {
  if (!error) return null
  return <Alert className="mb-4" status="danger"><Alert.Indicator><AlertCircle className="size-4" /></Alert.Indicator><Alert.Content><Alert.Title>请求未完成</Alert.Title><Alert.Description>{errorMessage(error)}</Alert.Description></Alert.Content></Alert>
}

export function OneTimeSecretPanel({ value, onClose }: { value: OneTimeSecret; onClose: () => void }) {
  const [copied, setCopied] = useState(false)
  const copy = async () => { await navigator.clipboard.writeText(value.secret); setCopied(true) }
  return <section className="mb-5 border border-[#b7d974] bg-[#f4fae7] p-4 text-accent-soft-foreground" style={{ borderRadius: 6 }}><div className="flex items-start justify-between gap-4"><div><h2 className="text-sm font-semibold">{value.title}</h2><code className="mt-1 block text-xs text-accent">{value.identifier}</code></div><Button variant="ghost" className="h-8 w-8 p-0" aria-label="关闭并丢弃 Secret" onClick={onClose}><X className="h-4 w-4" /></Button></div><div className="mt-4 flex gap-2"><input className="min-h-10 w-full rounded-md border border-field-border bg-field px-3 text-sm text-foreground outline-none transition focus:border-focus focus:ring-2 focus:ring-focus/20 bg-white font-mono text-xs" readOnly value={value.secret} /><Button variant="secondary" className="shrink-0" onClick={() => void copy()}>{copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}{copied ? '已复制' : '复制'}</Button></div><p className="mt-2 text-xs">关闭后不会再次显示。</p></section>
}

function Detail({ label, value, mono }: { label: string; value: string; mono?: boolean }) { return <div className="flex items-start justify-between gap-3 border-b border-separator pb-3 last:border-0"><dt className="text-muted">{label}</dt><dd className={cn('break-all text-right font-medium text-foreground', mono && 'font-mono text-xs')}>{value}</dd></div> }
function EmptyState({ icon: Icon, title, description, action }: { icon: typeof Search; title: string; description: string; action?: React.ReactNode }) { return <div className="flex min-h-[260px] flex-col items-center justify-center px-6 py-10 text-center"><span className="grid h-11 w-11 place-items-center rounded-lg border border-separator bg-default text-accent"><Icon className="h-5 w-5" /></span><h2 className="mt-4 text-base font-semibold text-foreground">{title}</h2><p className="mt-1 max-w-sm text-sm leading-6 text-muted">{description}</p>{action && <div className="mt-5">{action}</div>}</div> }
function Badge({ children, tone }: { children: React.ReactNode; tone: 'positive' | 'neutral' | 'danger' | 'warning' }) {
  const color = { positive: 'success', neutral: 'default', danger: 'danger', warning: 'warning' }[tone] as 'success' | 'default' | 'danger' | 'warning'
  const pendingDelete = children === 'delete_pending'
  const content = pendingDelete ? <span className="inline-flex items-center gap-1"><LoaderCircle className="size-3 animate-spin" />正在删除</span> : children
  return <Chip color={color} size="sm" variant="soft"><Chip.Label>{content}</Chip.Label></Chip>
}

function HomePage() {
  const applications = useQuery({ queryKey: ['applications'], queryFn: api.getApplications })
  const queryClient = useQueryClient()
  const [name, setName] = useState('')
  const create = useMutation({ mutationFn: () => api.createApplication(name.trim()), onSuccess: (application) => queryClient.setQueryData<Application[]>(['applications'], (current = []) => [...current, application]) })
  if (applications.isLoading) return <LoadingScreen />
  if (applications.error) return <AuthFormShell eyebrow="会话校验失败" title="无法加载应用" description="登录成功，但无法读取应用列表。请重试，或重新登录以刷新会话。">
    <MutationError error={applications.error} />
    <div className="mt-6 grid gap-3 sm:grid-cols-2">
      <Button variant="primary" isDisabled={applications.isFetching} onClick={() => void applications.refetch()}>{applications.isFetching && <LoaderCircle className="h-4 w-4 animate-spin" />}重试</Button>
      <LinkButton variant="secondary" to="/login">重新登录</LinkButton>
    </div>
  </AuthFormShell>
  if (applications.data?.[0]) return <Navigate to={appPath(applications.data[0].appId, 'dashboard')} replace />
  return <div className="grid min-h-screen place-items-center bg-background p-5"><Card variant="default" className="w-full max-w-md"><Card.Content className="p-7"><Logo /><h1 className="mt-9 text-xl font-semibold text-foreground">创建第一个应用</h1><p className="mt-2 text-sm leading-6 text-muted">应用用于隔离 Bucket、对象、密钥和 Webhook。</p><form className="mt-6 space-y-4" onSubmit={(event) => { event.preventDefault(); if (name.trim()) create.mutate() }}><label><span className="mb-2 block text-sm font-medium">应用名称</span><Input fullWidth autoFocus maxLength={128} value={name} onChange={(event) => setName(event.target.value)} /></label><MutationError error={applications.error ?? create.error} /><Button type="submit" variant="primary" className="w-full" isDisabled={create.isPending || !name.trim()}>{create.isPending && <LoaderCircle className="h-4 w-4 animate-spin" />}创建应用</Button></form></Card.Content></Card></div>
}

function AdminPage() {
  api.setApplication(undefined)
  const [tab, setTab] = useState<'users' | 'applications' | 'jobs' | 'storage' | 'settings' | 'audit'>('users')
  const [quotaEditor, setQuotaEditor] = useState<AdminApplication | null>(null)
  const users = useQuery({ queryKey: ['admin', 'users'], queryFn: api.getAdminUsers })
  const applications = useQuery({ queryKey: ['admin', 'applications'], queryFn: api.getAdminApplications })
  const jobs = useQuery({ queryKey: ['admin', 'jobs'], queryFn: api.getAdminJobs })
  const storage = useQuery({ queryKey: ['admin', 'storage'], queryFn: api.getAdminStorage })
  const systemSettings = useQuery({ queryKey: ['admin', 'settings'], queryFn: api.getAdminSystemSettings })
  const audit = useQuery({ queryKey: ['admin', 'audit'], queryFn: api.getAdminAudit })
  const queryClient = useQueryClient()
  const statusMutation = useMutation({ mutationFn: ({ userId, status }: { userId: string; status: 'active' | 'suspended' }) => api.updateAdminUserStatus(userId, status), onSuccess: async () => { await Promise.all([queryClient.invalidateQueries({ queryKey: ['admin', 'users'] }), queryClient.invalidateQueries({ queryKey: ['admin', 'audit'] })]) } })
  const quotaMutation = useMutation({ mutationFn: ({ applicationId, quotaBytes }: { applicationId: string; quotaBytes: number }) => api.updateAdminApplicationQuota(applicationId, quotaBytes), onSuccess: async () => { setQuotaEditor(null); await Promise.all([queryClient.invalidateQueries({ queryKey: ['admin', 'applications'] }), queryClient.invalidateQueries({ queryKey: ['admin', 'storage'] }), queryClient.invalidateQueries({ queryKey: ['admin', 'audit'] })]) } })
  const settingsMutation = useMutation({ mutationFn: api.updateAdminSystemSettings, onSuccess: (settings) => queryClient.setQueryData(['admin', 'settings'], settings) })
  const pendingJobs = jobs.data?.filter((job) => job.state === 'pending' || job.state === 'running').length
  const failedJobs = jobs.data?.filter((job) => job.state === 'failed').length
  const error = users.error ?? applications.error ?? jobs.error ?? storage.error ?? systemSettings.error ?? audit.error ?? statusMutation.error ?? quotaMutation.error
  const tabs: Array<{ id: typeof tab; label: string }> = [{ id: 'users', label: '用户' }, { id: 'applications', label: '应用' }, { id: 'jobs', label: '后台任务' }, { id: 'storage', label: '存储' }, { id: 'settings', label: '系统设置' }, { id: 'audit', label: '审计日志' }]
  const loading = <div className="grid min-h-56 place-items-center"><div className="flex items-center gap-3 text-sm text-muted"><Spinner aria-label="加载管理数据" color="accent" size="sm" />正在加载</div></div>
  const empty = (message: string) => <div className="grid min-h-56 place-items-center px-5 text-center"><div><p className="text-sm font-medium text-foreground">暂无数据</p><p className="mt-1 text-xs text-muted">{message}</p></div></div>
  const metricBorders = ['border-b border-separator sm:border-r xl:border-b-0', 'border-b border-separator xl:border-b-0 xl:border-r', 'border-b border-separator sm:border-b-0 sm:border-r', '']

  return <div className="min-h-screen bg-background">
    <header className="sticky top-0 z-30 border-b border-separator bg-surface/95 backdrop-blur">
      <div className="mx-auto flex h-16 max-w-[1760px] items-center justify-between px-5 sm:px-8">
        <div className="flex items-center gap-3"><Logo /><span className="hidden h-5 w-px bg-separator sm:block" /><span className="hidden text-xs font-medium text-muted sm:block">系统管理</span></div>
        <LinkButton variant="secondary" to="/"><ArrowRight className="h-4 w-4 rotate-180" />返回控制台</LinkButton>
      </div>
    </header>
    <main className="mx-auto max-w-[1760px] px-5 py-5 sm:px-8 sm:py-6">
      <MutationError error={error} />

      <section aria-label="系统概览" className="overflow-hidden rounded-lg border border-separator bg-surface shadow-sm">
        <header className="border-b border-separator px-5 py-4"><h1 className="text-base font-semibold">Admin</h1><p className="mt-1 text-xs text-muted">跨应用查看用户、资源、后台任务和审计记录。</p></header>
        <div className="grid sm:grid-cols-2 xl:grid-cols-4"><AdminMetric className={metricBorders[0]} label="应用" value={applications.data?.length} /><AdminMetric className={metricBorders[1]} label="用户" value={users.data?.length} /><AdminMetric className={metricBorders[2]} label="待处理任务" value={pendingJobs} /><AdminMetric className={metricBorders[3]} label="失败任务" value={failedJobs} caution /></div>
      </section>

      <section className="mt-4 overflow-hidden rounded-lg border border-separator bg-surface shadow-sm">
        <div role="tablist" aria-label="Admin 视图" className="flex min-h-14 gap-1 overflow-x-auto border-b border-separator px-3 pt-2 sm:px-5">
          {tabs.map((item) => <button key={item.id} type="button" role="tab" aria-selected={tab === item.id} className={cn('relative shrink-0 px-3 pb-2 text-sm font-medium text-muted transition-colors hover:text-foreground', tab === item.id && 'text-accent after:absolute after:inset-x-3 after:bottom-0 after:h-0.5 after:rounded-full after:bg-accent')} onClick={() => setTab(item.id)}>{item.label}</button>)}
        </div>

        {tab === 'users' && <div role="tabpanel">
          <div className="flex items-center justify-between gap-4 border-b border-separator px-5 py-4"><div><h2 className="text-sm font-semibold">用户状态</h2><p className="mt-1 text-xs text-muted">停用用户会同时撤销其现有 Session。</p></div><span className="shrink-0 text-xs tabular-nums text-muted">{users.data ? `共 ${users.data.length} 位` : '--'}</span></div>
          {users.isLoading ? loading : users.data?.length ? <div className="overflow-x-auto"><table className="w-full min-w-[820px]"><thead><tr className="table-head"><th className="px-5 py-3">邮箱</th><th className="px-5 py-3">角色</th><th className="px-5 py-3">状态</th><th className="px-5 py-3">创建时间</th><th className="px-5 py-3 text-right">操作</th></tr></thead><tbody className="divide-y divide-separator">{users.data.map((user) => <tr className="text-sm transition-colors hover:bg-default-soft" key={user.id}><td className="px-5 py-3.5 font-medium text-foreground">{user.email}</td><td className="px-5 py-3.5"><Badge tone={user.systemRole === 'admin' ? 'positive' : 'neutral'}>{user.systemRole}</Badge></td><td className="px-5 py-3.5"><Badge tone={user.status === 'active' ? 'positive' : user.status === 'suspended' ? 'danger' : 'warning'}>{user.status}</Badge></td><td className="px-5 py-3.5 text-muted">{formatDateTime(user.createdAt)}</td><td className="px-5 py-3.5 text-right">{user.status !== 'pending_verification' && <Button variant={user.status === 'active' ? 'danger-soft' : 'secondary'} className="h-8 text-xs" isDisabled={statusMutation.isPending} onClick={() => statusMutation.mutate({ userId: user.id, status: user.status === 'active' ? 'suspended' : 'active' })}>{user.status === 'active' ? '停用' : '恢复'}</Button>}</td></tr>)}</tbody></table></div> : empty('用户创建后会显示在这里。')}
        </div>}

        {tab === 'applications' && <div role="tabpanel">
          <div className="flex items-center justify-between gap-4 border-b border-separator px-5 py-4"><div><h2 className="text-sm font-semibold">应用与配额</h2><p className="mt-1 text-xs text-muted">比较所有应用的配额、实际用量和预占空间。</p></div><span className="shrink-0 text-xs tabular-nums text-muted">{applications.data ? `共 ${applications.data.length} 个` : '--'}</span></div>
          {applications.isLoading ? loading : applications.data?.length ? <div className="overflow-x-auto"><table className="w-full min-w-[860px]"><thead><tr className="table-head"><th className="px-5 py-3">应用</th><th className="px-5 py-3">AppId</th><th className="px-5 py-3">存储用量</th><th className="px-5 py-3">预占</th><th className="px-5 py-3">配额</th><th aria-label="操作" className="w-20 px-3 py-3" /></tr></thead><tbody className="divide-y divide-separator">{applications.data.map((application) => <tr className="text-sm transition-colors hover:bg-default-soft" key={application.id}><td className="px-5 py-3.5 font-medium text-foreground">{application.name}</td><td className="px-5 py-3.5 font-mono text-xs text-muted">{application.appId}</td><td className="px-5 py-3.5 tabular-nums text-muted">{formatBytes(application.usedBytes)}</td><td className="px-5 py-3.5 tabular-nums text-muted">{formatBytes(application.reservedBytes)}</td><td className="px-5 py-3.5 tabular-nums text-muted">{formatBytes(application.quotaBytes)}</td><td className="px-3 py-3.5 text-right"><Button isIconOnly size="sm" variant="ghost" aria-label={`调整 ${application.name} 配额`} onClick={() => { quotaMutation.reset(); setQuotaEditor(application) }}><Pencil className="size-3.5" /></Button></td></tr>)}</tbody></table></div> : empty('应用创建后会显示在这里。')}
        </div>}

        {tab === 'jobs' && <div role="tabpanel">
          <div className="flex items-center justify-between gap-4 border-b border-separator px-5 py-4"><div><h2 className="text-sm font-semibold">后台任务</h2><p className="mt-1 text-xs text-muted">查看批量操作的持久化状态、执行进度和重试次数。</p></div><span className="shrink-0 text-xs tabular-nums text-muted">{jobs.data ? `共 ${jobs.data.length} 个` : '--'}</span></div>
          {jobs.isLoading ? loading : jobs.data?.length ? <div className="overflow-x-auto"><table className="w-full min-w-[940px]"><thead><tr className="table-head"><th className="px-5 py-3">Job</th><th className="px-5 py-3">操作</th><th className="px-5 py-3">状态</th><th className="px-5 py-3">进度</th><th className="px-5 py-3">重试</th><th className="px-5 py-3">更新时间</th></tr></thead><tbody className="divide-y divide-separator">{jobs.data.map((job) => { const completed = job.succeededItems + job.failedItems; const progress = job.totalItems ? Math.round((completed / job.totalItems) * 100) : 0; return <tr className="text-sm transition-colors hover:bg-default-soft" key={job.id}><td className="max-w-72 px-5 py-3.5"><code className="block truncate text-xs text-foreground" title={job.id}>{job.id}</code></td><td className="px-5 py-3.5 font-medium">{job.action}</td><td className="px-5 py-3.5"><Badge tone={job.state === 'completed' ? 'positive' : job.state === 'failed' ? 'danger' : 'neutral'}>{job.state}</Badge></td><td className="w-48 px-5 py-3.5"><div className="flex items-center justify-between gap-3 text-xs tabular-nums"><span>{completed} / {job.totalItems}</span><span className="text-muted">{progress}%</span></div><div className="mt-1.5 h-1 overflow-hidden rounded-full bg-default"><div className={cn('h-full rounded-full', job.state === 'failed' ? 'bg-danger' : 'bg-accent')} style={{ width: `${progress}%` }} /></div></td><td className="px-5 py-3.5 tabular-nums text-muted">{job.attemptCount} / {job.maxAttempts}</td><td className="px-5 py-3.5 text-muted">{formatDateTime(job.updatedAt)}</td></tr> })}</tbody></table></div> : empty('后台任务产生后会显示在这里。')}
        </div>}

        {tab === 'storage' && <div role="tabpanel">
          <div className="border-b border-separator px-5 py-4"><h2 className="text-sm font-semibold">存储概览</h2><p className="mt-1 text-xs text-muted">全局存储用量、对象规模和当前预占空间。</p></div>
          {storage.isLoading ? loading : storage.data ? <div className="grid sm:grid-cols-2 xl:grid-cols-4"><AdminMetric className={metricBorders[0]} label="已用配额" value={storage.data.usedBytes} formatter={formatBytes} /><AdminMetric className={metricBorders[1]} label="对象数" value={storage.data.mediaObjects} /><AdminMetric className={metricBorders[2]} label="磁盘可用" value={storage.data.diskAvailableBytes} formatter={formatBytes} /><AdminMetric className={metricBorders[3]} label="预占空间" value={storage.data.reservedBytes} formatter={formatBytes} /></div> : empty('存储指标暂不可用。')}
        </div>}

        {tab === 'settings' && <div role="tabpanel">
          <div className="border-b border-separator px-5 py-4"><h2 className="text-sm font-semibold">传输设置</h2><p className="mt-1 text-xs text-muted">限制每个对象响应的下载速度；多个下载仍可并行。</p></div>
          {systemSettings.isLoading ? loading : systemSettings.data ? <AdminSystemSettingsPanel value={systemSettings.data} pending={settingsMutation.isPending} error={settingsMutation.error} onSave={(bytesPerSecond) => settingsMutation.mutate(bytesPerSecond)} /> : empty('系统设置暂不可用。')}
        </div>}

        {tab === 'audit' && <div role="tabpanel">
          <div className="flex items-center justify-between gap-4 border-b border-separator px-5 py-4"><div><h2 className="text-sm font-semibold">审计日志</h2><p className="mt-1 text-xs text-muted">跨应用的不可变管理操作记录。</p></div><span className="shrink-0 text-xs tabular-nums text-muted">{audit.data ? `共 ${audit.data.length} 条` : '--'}</span></div>
          {audit.isLoading ? loading : audit.data?.length ? <div className="overflow-x-auto"><table className="w-full min-w-[1040px]"><thead><tr className="table-head"><th className="px-5 py-3">时间</th><th className="px-5 py-3">操作</th><th className="px-5 py-3">Actor</th><th className="px-5 py-3">目标</th><th className="px-5 py-3">Request ID</th></tr></thead><tbody className="divide-y divide-separator">{audit.data.map((event) => <tr className="text-sm transition-colors hover:bg-default-soft" key={event.id}><td className="whitespace-nowrap px-5 py-3.5 text-muted">{formatDateTime(event.createdAt)}</td><td className="px-5 py-3.5 font-medium text-foreground">{event.action}</td><td className="max-w-64 px-5 py-3.5"><span className="block text-xs text-muted">{event.actorType}</span><span className="mt-0.5 block truncate" title={event.actorId}>{event.actorId}</span></td><td className="max-w-64 px-5 py-3.5"><span className="block text-xs text-muted">{event.targetType}</span><span className="mt-0.5 block truncate" title={event.targetId}>{event.targetId}</span></td><td className="max-w-64 px-5 py-3.5"><code className="block truncate text-xs text-muted" title={event.requestId}>{event.requestId}</code></td></tr>)}</tbody></table></div> : empty('管理操作发生后会写入不可变审计记录。')}
        </div>}
      </section>
      {quotaEditor && <ApplicationQuotaEditor application={quotaEditor} pending={quotaMutation.isPending} error={quotaMutation.error} onClose={() => setQuotaEditor(null)} onSave={(quotaBytes) => quotaMutation.mutate({ applicationId: quotaEditor.id, quotaBytes })} />}
    </main>
  </div>
}

function AdminSystemSettingsPanel({ value, pending, error, onSave }: { value: AdminSystemSettings; pending: boolean; error: unknown; onSave: (bytesPerSecond: number | null) => void }) {
  const mebibyte = 1024 ** 2
  const [limited, setLimited] = useState(value.downloadBytesPerSecond !== null)
  const [rateMiB, setRateMiB] = useState(String((value.downloadBytesPerSecond ?? 32 * mebibyte) / mebibyte))
  useEffect(() => {
    setLimited(value.downloadBytesPerSecond !== null)
    if (value.downloadBytesPerSecond !== null) setRateMiB(String(value.downloadBytesPerSecond / mebibyte))
  }, [value])
  const parsedRate = Number(rateMiB)
  const valid = !limited || (Number.isInteger(parsedRate) && parsedRate >= 1 && parsedRate <= 1024)
  const bytesPerSecond = limited && valid ? parsedRate * mebibyte : null
  const unchanged = bytesPerSecond === value.downloadBytesPerSecond
  return <form className="max-w-3xl space-y-5 px-5 py-5" onSubmit={(event) => { event.preventDefault(); if (valid) onSave(bytesPerSecond) }}>
    <div className="flex flex-col gap-4 rounded-md border border-separator bg-default-soft px-4 py-4 sm:flex-row sm:items-center sm:justify-between">
      <div><p className="text-sm font-medium text-foreground">限制单文件下载速度</p><p className="mt-1 text-xs leading-5 text-muted">按单个 HTTP 响应限速，Range、原图和图片 Variant 使用同一设置。</p></div>
      <Switch isSelected={limited} onChange={setLimited}><Switch.Content><Switch.Control><Switch.Thumb /></Switch.Control><span className="sr-only">限制单文件下载速度</span></Switch.Content></Switch>
    </div>
    <label className="block max-w-sm"><span className="mb-1.5 block text-xs font-medium text-muted">每个响应（MiB/s）</span><Input fullWidth aria-label="每个响应（MiB/s）" type="number" min="1" max="1024" step="1" disabled={!limited} value={rateMiB} onChange={(event) => setRateMiB(event.target.value)} /><span className="mt-1.5 block text-xs text-muted">可设置 1–1024 MiB/s；关闭开关后不限制下载速度。</span></label>
    <MutationError error={error} />
    <div className="flex items-center justify-between gap-4 border-t border-separator pt-4"><p className="text-xs text-muted">上次更新：{formatDateTime(value.updatedAt)}</p><Button type="submit" variant="primary" isDisabled={pending || !valid || unchanged}>{pending && <LoaderCircle className="size-4 animate-spin" />}保存系统设置</Button></div>
  </form>
}

function ApplicationQuotaEditor({ application, pending, error, onClose, onSave }: { application: AdminApplication; pending: boolean; error: unknown; onClose: () => void; onSave: (quotaBytes: number) => void }) {
  const gibibyte = 1024 ** 3
  const minimumBytes = application.usedBytes + application.reservedBytes
  const minimumGiB = Math.ceil((minimumBytes / gibibyte) * 100) / 100
  const [quotaGiB, setQuotaGiB] = useState(String(application.quotaBytes / gibibyte))
  const parsedGiB = Number(quotaGiB)
  const quotaBytes = Math.round(parsedGiB * gibibyte)
  const valid = Number.isFinite(parsedGiB) && parsedGiB >= 0 && Number.isSafeInteger(quotaBytes) && quotaBytes >= minimumBytes
  return <Modal title={`调整 ${application.name} 配额`} onClose={onClose}><form onSubmit={(event) => { event.preventDefault(); if (valid) onSave(quotaBytes) }} className="space-y-4">
    <div className="grid gap-3 sm:grid-cols-2"><Detail label="已用" value={formatBytes(application.usedBytes)} /><Detail label="预占" value={formatBytes(application.reservedBytes)} /></div>
    <label><span className="mb-1.5 block text-xs font-medium">总配额（GiB）</span><Input fullWidth aria-label="总配额（GiB）" type="number" min={minimumGiB} step="0.01" value={quotaGiB} onChange={(event) => setQuotaGiB(event.target.value)} /><span className="mt-1.5 block text-xs text-muted">最低 {minimumGiB} GiB，不能低于已用与预占空间之和。</span></label>
    <Alert status="accent"><Alert.Indicator><CircleHelp className="size-4" /></Alert.Indicator><Alert.Content><Alert.Title>Application 总额度</Alert.Title><Alert.Description>该 Application 下所有 Bucket 共享此配额；Bucket 页面展示各自的实际用量。</Alert.Description></Alert.Content></Alert>
    <MutationError error={error} />
    <ModalActions onCancel={onClose} pending={pending} submitLabel="保存配额" disabled={!valid || quotaBytes === application.quotaBytes} />
  </form></Modal>
}

function AdminMetric({ label, value, caution = false, formatter, className }: { label: string; value: number | null | undefined; caution?: boolean; formatter?: (value: number) => string; className?: string }) { const available = typeof value === 'number'; const hasCaution = caution && available && value > 0; return <article className={cn('min-w-0 bg-surface px-5 py-4', className)}><div className="flex items-center justify-between gap-3"><p className="text-xs font-medium text-muted">{label}</p><span className={cn('h-2 w-2 rounded-full', !available ? 'bg-default' : hasCaution ? 'bg-danger' : 'bg-success')} /></div><p className={cn('mt-2 text-2xl font-semibold tabular-nums text-foreground', hasCaution && 'text-danger')}>{available ? (formatter ? formatter(value) : value.toLocaleString()) : '--'}</p><p className="mt-0.5 text-[11px] text-muted">{available ? '实时数据' : '暂不可用'}</p></article> }

export function App() {
  return <Routes><Route path="/login" element={<LoginPage />} /><Route path="/register" element={<RegisterPage />} /><Route path="/verify-email" element={<VerifyEmailPage />} /><Route path="/forgot-password" element={<ForgotPasswordPage />} /><Route path="/reset-password" element={<ResetPasswordPage />} /><Route path="/app/:appId/*" element={<RequireAuth><ConsoleShellV3 /></RequireAuth>} /><Route path="/admin/*" element={<RequireAdmin><AdminPage /></RequireAdmin>} /><Route path="/" element={<RequireAuth><HomePage /></RequireAuth>} /><Route path="*" element={<Navigate to="/" replace />} /></Routes>
}
