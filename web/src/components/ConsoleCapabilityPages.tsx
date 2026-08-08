import { Chip } from '@heroui/react'
import { buttonVariants } from '@heroui/styles'
import {
  ArrowRight,
  Bot,
  Boxes,
  CheckCircle2,
  Clock3,
  FileArchive,
  FileImage,
  FileSpreadsheet,
  FileText,
  Film,
  ImageIcon,
  Layers3,
  LockKeyhole,
  ScanSearch,
  ShieldCheck,
  Sparkles,
  Tags,
  Waypoints,
} from 'lucide-react'
import type { ComponentType } from 'react'
import { NavLink, useParams } from 'react-router-dom'

type FeatureState = 'available' | 'foundation' | 'planned'

type FeatureItem = {
  title: string
  description: string
  icon: ComponentType<{ className?: string }>
  state?: FeatureState
}

const stateLabel: Record<FeatureState, string> = {
  available: '可用',
  foundation: '基础已具备',
  planned: 'S3 重构后接入',
}

const stateTone: Record<FeatureState, 'success' | 'accent' | 'default'> = {
  available: 'success',
  foundation: 'accent',
  planned: 'default',
}

function appPath(appId: string, path: string) {
  return `/app/${encodeURIComponent(appId)}/${path}`
}

function FeatureCard({ item, defaultState }: { item: FeatureItem; defaultState: FeatureState }) {
  const Icon = item.icon
  const state = item.state ?? defaultState
  return (
    <article className="group min-h-44 rounded-lg border border-separator bg-surface p-5 shadow-sm transition hover:-translate-y-0.5 hover:border-accent/35 hover:shadow-md">
      <div className="flex items-start justify-between gap-4">
        <span className="grid size-10 shrink-0 place-items-center rounded-lg bg-accent-soft text-accent-soft-foreground">
          <Icon className="size-5" />
        </span>
        <Chip size="sm" variant="soft" color={stateTone[state]}><Chip.Label>{stateLabel[state]}</Chip.Label></Chip>
      </div>
      <h2 className="mt-5 text-sm font-semibold text-foreground">{item.title}</h2>
      <p className="mt-2 text-xs leading-5 text-muted">{item.description}</p>
    </article>
  )
}

function CapabilityPage({
  eyebrow,
  title,
  description,
  state,
  items,
  aside,
}: {
  eyebrow: string
  title: string
  description: string
  state: FeatureState
  items: FeatureItem[]
  aside?: React.ReactNode
}) {
  const { appId = '' } = useParams()
  return (
    <div className="space-y-5">
      <section className="overflow-hidden rounded-xl border border-separator bg-surface shadow-sm">
        <div className="relative overflow-hidden px-5 py-7 sm:px-7 sm:py-9">
          <div aria-hidden="true" className="absolute -right-16 -top-20 size-64 rounded-full bg-accent-soft blur-3xl" />
          <div className="relative max-w-3xl">
            <div className="flex flex-wrap items-center gap-2">
              <p className="eyebrow">{eyebrow}</p>
              <Chip size="sm" variant="soft" color={stateTone[state]}><Chip.Label>{stateLabel[state]}</Chip.Label></Chip>
            </div>
            <h1 className="mt-3 text-2xl font-semibold tracking-tight text-foreground sm:text-3xl">{title}</h1>
            <p className="mt-3 max-w-2xl text-sm leading-6 text-muted">{description}</p>
            <div className="mt-6 flex flex-wrap gap-2">
              <NavLink to={appPath(appId, 'objects')} className={buttonVariants({ variant: 'primary' })}>打开文件浏览器<ArrowRight className="size-4" /></NavLink>
              <NavLink to={appPath(appId, 'buckets')} className={buttonVariants({ variant: 'secondary' })}>管理 Buckets</NavLink>
            </div>
          </div>
        </div>
        {aside}
      </section>
      <section className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
        {items.map((item) => <FeatureCard key={item.title} item={item} defaultState={state} />)}
      </section>
    </div>
  )
}

export function PreviewCenterPage() {
  return <CapabilityPage
    eyebrow="Preview Studio"
    title="不下载，也能看懂几乎所有文件"
    description="预览不是对象存储旁边的附属按钮，而是这个产品的核心工作台。文件按需加载、插件隔离，原始对象保持不可变。"
    state="available"
    items={[
      { title: '图片与设计资源', description: '常见栅格图、SVG、HEIC、TIFF 与多种相机/设计输出可直接查看。', icon: FileImage },
      { title: '文档与演示', description: 'PDF、Office 文档、Markdown、代码和纯文本使用适合内容的阅读界面。', icon: FileText },
      { title: '表格与数据库', description: '电子表格和 SQLite 在浏览器沙箱中只读探索，无需上传到第三方转换服务。', icon: FileSpreadsheet },
      { title: '压缩包', description: '浏览 ZIP、7z 等归档目录，并只提取需要查看的条目。', icon: FileArchive },
      { title: '音视频与三维', description: '媒体播放与常见三维格式共用统一预览窗口和对象详情。', icon: Film },
      { title: '安全与渐进加载', description: '大文件采用范围读取、大小策略和 Worker 隔离，失败时提供清晰降级。', icon: ShieldCheck },
    ]}
  />
}

export function VariantCenterPage() {
  return <CapabilityPage
    eyebrow="Variants"
    title="从一份原图生成可重复的交付版本"
    description="Variant 由不可变源对象、规范化参数和处理器版本共同寻址。相同请求复用结果，并通过租约与 fencing 避免重复处理。"
    state="available"
    aside={<div className="grid border-t border-separator bg-default-soft sm:grid-cols-3">
      <VariantFact label="变换" value="Resize · Fit · Crop" />
      <VariantFact label="输出" value="WebP · AVIF · JPEG · PNG" />
      <VariantFact label="一致性" value="Lease · Fence · Cache" />
    </div>}
    items={[
      { title: '交互式图片 Variant', description: '在预览侧栏调整尺寸、适配方式、质量、格式和模糊参数。', icon: ImageIcon },
      { title: '不可变缓存键', description: '源文件版本变化后自然生成新缓存空间，不会把旧衍生物错误复用到新内容。', icon: Layers3 },
      { title: '应用交付链接', description: '公开对象与私有签名对象使用同一套 Variant 语义和缓存策略。', icon: Waypoints },
      { title: '视频 Variant', description: '转码耗时较长，后续由独立服务和队列处理；当前不会在 Web 请求中同步执行。', icon: Film, state: 'planned' },
      { title: 'AI 派生能力', description: '为标注、摘要、OCR、Embedding 和生成式编辑预留统一的派生产物模型。', icon: Bot, state: 'planned' },
      { title: '处理可观测性', description: '任务、失败原因、处理器版本与缓存命中将进入统一运营视图。', icon: Sparkles, state: 'foundation' },
    ]}
  />
}

function VariantFact({ label, value }: { label: string; value: string }) {
  return <div className="border-b border-separator px-5 py-4 last:border-0 sm:border-b-0 sm:border-r sm:last:border-r-0"><p className="text-[11px] font-medium text-muted">{label}</p><p className="mt-1 text-sm font-semibold text-foreground">{value}</p></div>
}

const governancePages = {
  policies: {
    eyebrow: 'Access Control',
    title: 'S3 Policy 与统一授权',
    description: '用 S3 Action、Resource 和 Condition 统一 SDK、控制台与 WebDAV 的授权语义，替换当前固定权限字符串。',
    items: [
      { title: '身份与授权分离', description: '先解析 Access Key 与签名身份，再用 Policy 独立判断操作权限。', icon: ShieldCheck },
      { title: '资源级规则', description: '支持 Bucket、Object、Prefix 与版本资源，并为条件键保留扩展点。', icon: Tags },
      { title: '协议共用', description: 'S3、JSON 控制面和 DAV 映射到同一动作模型，不维护三套权限判断。', icon: Waypoints },
    ],
  },
  versioning: {
    eyebrow: 'Data Protection',
    title: 'Bucket Versioning',
    description: '对象由逻辑 Key 与不可变版本组成，支持 Enabled、Suspended、null version 和删除标记。',
    items: [
      { title: '不可变版本', description: '每次写入创建新版本，当前对象只是可见版本指针。', icon: Layers3 },
      { title: '删除标记', description: '普通删除写入 delete marker，指定 versionId 才删除目标版本。', icon: Clock3 },
      { title: '版本级预览', description: '预览和 Variant 绑定 object_version_id，历史内容不会漂移。', icon: ScanSearch },
    ],
  },
  lifecycle: {
    eyebrow: 'Data Management',
    title: 'Lifecycle Rules',
    description: '生命周期将按 S3 Rule、Filter 和 Action 建模，覆盖当前版本、历史版本、删除标记和 Multipart 清理。',
    items: [
      { title: '规则与筛选器', description: '按 Prefix、Tag、大小和状态匹配对象，配置可独立验证。', icon: Tags },
      { title: '版本感知', description: '当前版本与非当前版本使用不同到期动作，避免误删可见对象。', icon: Layers3 },
      { title: '安全执行', description: '执行器检查 Object Lock、任务幂等和存储提交状态后再改变可见性。', icon: CheckCircle2 },
    ],
  },
  'object-lock': {
    eyebrow: 'Compliance',
    title: 'Object Lock',
    description: '在版本级别支持 Retention、Legal Hold、Governance 和 Compliance，删除与覆盖共用一套检查顺序。',
    items: [
      { title: 'Retention', description: '为对象版本设置保留期限，在期限内拒绝不符合策略的删除。', icon: Clock3 },
      { title: 'Legal Hold', description: '与时间无关的法律保留状态，只有具备独立权限的主体可以改变。', icon: LockKeyhole },
      { title: '治理绕过', description: 'Governance 绕过需要显式请求头与独立动作授权；Compliance 不可绕过。', icon: ShieldCheck },
    ],
  },
} satisfies Record<string, { eyebrow: string; title: string; description: string; items: FeatureItem[] }>

export type GovernancePageKind = keyof typeof governancePages

export function GovernanceFeaturePage({ kind }: { kind: GovernancePageKind }) {
  const page = governancePages[kind]
  return <CapabilityPage {...page} state="planned" />
}

export function ObjectsOverviewLink() {
  const { appId = '' } = useParams()
  return <NavLink to={appPath(appId, 'objects')} className="inline-flex items-center gap-2 text-sm font-medium text-accent hover:underline"><Boxes className="size-4" />返回对象浏览器</NavLink>
}
