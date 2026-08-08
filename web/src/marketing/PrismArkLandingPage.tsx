import { Card, Chip } from '@heroui/react'
import {
  ArrowRight,
  Blocks,
  Bot,
  Box,
  Braces,
  Check,
  ChevronRight,
  Cloud,
  Code2,
  Database,
  Eye,
  FileArchive,
  FileImage,
  FileSpreadsheet,
  FileText,
  Folder,
  FolderOpen,
  Gauge,
  GitBranch,
  HardDrive,
  Image as ImageIcon,
  Layers3,
  LockKeyhole,
  Maximize2,
  Menu,
  MousePointer2,
  Network,
  PackageOpen,
  PanelRight,
  Play,
  Search,
  Server,
  ShieldCheck,
  Sparkles,
  TerminalSquare,
  WandSparkles,
  Waypoints,
  type LucideIcon,
} from 'lucide-react'
import type { ReactNode } from 'react'

import { ThemeToggle } from '../components/ThemeToggle'

import { PrismArkBrand, PrismArkMark } from './PrismArkBrand'

export const PRISMARK_PAGE_META = {
  title: 'PrismArk 万象仓｜S3 对象存储、WebDAV 与全格式文件预览',
  description:
    'PrismArk 是面向团队与开发者的自托管对象存储和内容预览平台，以 S3 为核心协议、WebDAV 为兼容层，提供全格式文件预览、Win11 风格文件浏览器和图片 Variant。',
} as const

export interface PrismArkLandingPageProps {
  consoleHref?: string
  docsHref?: string
  sourceHref?: string
}

const navigation = [
  { label: '产品能力', href: '#capabilities' },
  { label: '文件体验', href: '#file-experience' },
  { label: '协议与架构', href: '#architecture' },
  { label: '产品路线', href: '#roadmap' },
] as const

const previewFormats = [
  { icon: FileImage, label: '图片与设计稿', detail: '常见位图、矢量图与图像元数据' },
  { icon: FileText, label: '文档与代码', detail: 'PDF、文本、Markdown、源代码与配置文件' },
  { icon: FileSpreadsheet, label: '表格与数据', detail: '电子表格、CSV、SQLite 与结构化数据' },
  { icon: FileArchive, label: '压缩与归档', detail: '直接查看目录结构，无需先下载解压' },
] as const

const schema = JSON.stringify({
  '@context': 'https://schema.org',
  '@type': 'SoftwareApplication',
  name: 'PrismArk',
  alternateName: '万象仓',
  applicationCategory: 'DeveloperApplication',
  operatingSystem: 'Linux, macOS, Windows, Web',
  description: PRISMARK_PAGE_META.description,
  featureList: [
    'S3-compatible object storage',
    'WebDAV compatibility layer',
    'Universal file preview',
    'Windows 11 style file browser',
    'Image variants',
    'Self-hosted deployment',
  ],
})

function SectionHeading({
  eyebrow,
  title,
  description,
  align = 'left',
}: {
  eyebrow: string
  title: string
  description: string
  align?: 'left' | 'center'
}) {
  return (
    <div className={align === 'center' ? 'mx-auto max-w-3xl text-center' : 'max-w-2xl'}>
      <p className="text-xs font-bold uppercase text-accent-soft-foreground">{eyebrow}</p>
      <h2 className="mt-3 text-3xl font-bold leading-tight text-foreground sm:text-4xl">{title}</h2>
      <p className="mt-4 text-base leading-7 text-muted sm:text-lg">{description}</p>
    </div>
  )
}

function PrimaryLink({ href, children }: { href: string; children: ReactNode }) {
  return (
    <a
      href={href}
      className="inline-flex min-h-11 items-center justify-center gap-2 rounded-lg bg-accent px-5 py-2.5 text-sm font-semibold text-accent-foreground shadow-sm transition hover:bg-accent-hover"
    >
      {children}
    </a>
  )
}

function SecondaryLink({ href, children }: { href: string; children: ReactNode }) {
  return (
    <a
      href={href}
      className="inline-flex min-h-11 items-center justify-center gap-2 rounded-lg border border-border bg-surface px-5 py-2.5 text-sm font-semibold text-foreground transition hover:bg-surface-hover"
    >
      {children}
    </a>
  )
}

function FeatureCard({
  icon: Icon,
  title,
  children,
  tag,
}: {
  icon: LucideIcon
  title: string
  children: ReactNode
  tag?: string
}) {
  return (
    <Card variant="default" className="h-full border-border bg-surface shadow-sm">
      <Card.Content className="p-6">
        <div className="flex items-start justify-between gap-4">
          <span className="grid size-11 place-items-center rounded-xl bg-accent-soft text-accent-soft-foreground">
            <Icon className="size-5" aria-hidden="true" />
          </span>
          {tag && (
            <Chip size="sm" variant="soft" color="accent">
              <Chip.Label>{tag}</Chip.Label>
            </Chip>
          )}
        </div>
        <h3 className="mt-5 text-lg font-semibold text-foreground">{title}</h3>
        <p className="mt-2 text-sm leading-6 text-muted">{children}</p>
      </Card.Content>
    </Card>
  )
}

function ExplorerMockup() {
  const files = [
    { icon: Folder, name: '品牌资产', meta: '24 个对象', tone: 'text-amber-500 bg-amber-500/10' },
    { icon: FileImage, name: 'hero-campaign.webp', meta: '2400 × 1600', tone: 'text-cyan-500 bg-cyan-500/10' },
    { icon: FileText, name: '产品说明书.pdf', meta: '18 页 · 3.2 MB', tone: 'text-rose-500 bg-rose-500/10' },
    { icon: FileSpreadsheet, name: 'content-index.xlsx', meta: '6 个工作表', tone: 'text-emerald-500 bg-emerald-500/10' },
  ] as const

  return (
    <div
      className="overflow-hidden rounded-2xl border border-border bg-surface shadow-[var(--overlay-shadow)]"
      aria-label="PrismArk 文件浏览器界面示意"
    >
      <div className="flex items-center gap-2 border-b border-separator bg-default-soft px-4 py-3">
        <span className="size-2.5 rounded-full bg-danger" />
        <span className="size-2.5 rounded-full bg-warning" />
        <span className="size-2.5 rounded-full bg-success" />
        <div className="ml-3 flex min-w-0 flex-1 items-center gap-2 rounded-md border border-border bg-surface px-3 py-1.5 text-[11px] text-muted">
          <HardDrive className="size-3.5" aria-hidden="true" />
          <span className="truncate">assets / launch /</span>
        </div>
        <Search className="size-4 text-muted" aria-hidden="true" />
      </div>

      <div className="grid min-h-[390px] grid-cols-[76px_minmax(0,1fr)] sm:grid-cols-[152px_minmax(0,1fr)]">
        <aside className="border-r border-separator bg-background-secondary/70 p-3" aria-label="示意导航">
          {[
            [FolderOpen, '文件'],
            [Eye, '最近预览'],
            [ImageIcon, 'Variants'],
          ].map(([Icon, label], index) => {
            const ItemIcon = Icon as LucideIcon
            return (
              <div
                className={`mb-1 flex items-center gap-2 rounded-md px-2 py-2 text-xs ${index === 0 ? 'bg-accent-soft text-accent-soft-foreground' : 'text-muted'}`}
                key={label as string}
              >
                <ItemIcon className="size-4 shrink-0" aria-hidden="true" />
                <span className="hidden sm:inline">{label as string}</span>
              </div>
            )
          })}
        </aside>

        <div className="min-w-0 p-3 sm:p-4">
          <div className="mb-4 flex items-center justify-between gap-3">
            <div>
              <p className="text-sm font-semibold text-foreground">Launch</p>
              <p className="text-[10px] text-muted">文件夹与对象统一浏览</p>
            </div>
            <div className="flex items-center gap-1 rounded-md border border-border bg-default-soft p-1 text-muted">
              <Blocks className="size-3.5 text-accent" aria-hidden="true" />
              <Menu className="size-3.5" aria-hidden="true" />
            </div>
          </div>

          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            {files.map(({ icon: Icon, name, meta, tone }, index) => (
              <article
                className={`group rounded-xl border p-3 ${index === 1 ? 'border-accent bg-accent-soft/40' : 'border-border bg-background'}`}
                key={name}
              >
                <div className={`grid aspect-[16/9] place-items-center rounded-lg ${tone}`}>
                  <Icon className="size-9" aria-hidden="true" />
                </div>
                <p className="mt-3 truncate text-xs font-medium text-foreground">{name}</p>
                <p className="mt-1 text-[10px] text-muted">{meta}</p>
              </article>
            ))}
          </div>
        </div>
      </div>
    </div>
  )
}

function PreviewMockup() {
  return (
    <div className="relative overflow-hidden rounded-2xl border border-border bg-[#0b1020] shadow-[var(--overlay-shadow)]">
      <div className="flex items-center justify-between border-b border-white/10 px-4 py-3 text-white">
        <div className="flex min-w-0 items-center gap-2">
          <FileImage className="size-4 shrink-0 text-cyan-300" aria-hidden="true" />
          <span className="truncate text-xs font-medium">campaign-master.psd</span>
          <span className="hidden text-[10px] text-white/45 sm:inline">image/vnd.adobe.photoshop</span>
        </div>
        <div className="flex gap-3 text-white/60">
          <Gauge className="size-4" aria-hidden="true" />
          <Maximize2 className="size-4" aria-hidden="true" />
        </div>
      </div>
      <div className="relative grid min-h-[340px] place-items-center overflow-hidden bg-[radial-gradient(circle_at_50%_35%,rgba(59,130,246,.28),transparent_45%)] p-8">
        <div className="absolute inset-0 opacity-30 [background-image:linear-gradient(rgba(255,255,255,.05)_1px,transparent_1px),linear-gradient(90deg,rgba(255,255,255,.05)_1px,transparent_1px)] [background-size:24px_24px]" />
        <div className="relative grid aspect-[4/3] w-full max-w-sm place-items-center overflow-hidden rounded-xl border border-white/15 bg-gradient-to-br from-indigo-500 via-blue-500 to-cyan-400 shadow-2xl">
          <div className="absolute -left-12 top-4 size-48 rounded-full bg-violet-400/55 blur-2xl" />
          <div className="absolute -right-10 bottom-0 size-48 rounded-full bg-cyan-200/60 blur-2xl" />
          <picture className="relative block size-32">
            <source srcSet="/brand/prismark-mark.webp" type="image/webp" />
            <img src="/brand/prismark-mark-512.png" alt="" aria-hidden="true" width="512" height="512" className="size-full object-contain" fetchPriority="high" decoding="async" />
          </picture>
        </div>
      </div>
      <div className="flex items-center justify-between border-t border-white/10 px-4 py-3 text-[10px] text-white/50">
        <span>无需下载即可检查内容</span>
        <span>原文件 · 42.8 MB</span>
      </div>
    </div>
  )
}

function ArchitectureNode({ icon: Icon, title, detail }: { icon: LucideIcon; title: string; detail: string }) {
  return (
    <div className="rounded-xl border border-border bg-surface p-4 text-center shadow-sm">
      <Icon className="mx-auto size-5 text-accent" aria-hidden="true" />
      <p className="mt-2 text-sm font-semibold text-foreground">{title}</p>
      <p className="mt-1 text-[11px] leading-5 text-muted">{detail}</p>
    </div>
  )
}

export function PrismArkLandingPage({
  consoleHref = '/login',
  docsHref = '#deployment',
  sourceHref,
}: PrismArkLandingPageProps) {
  const effectiveSourceHref = sourceHref ?? docsHref
  return (
    <div className="min-h-screen overflow-x-hidden bg-background text-foreground">
      <script type="application/ld+json" dangerouslySetInnerHTML={{ __html: schema }} />
      <a
        href="#main-content"
        className="fixed left-4 top-4 z-[100] -translate-y-24 rounded-md bg-accent px-4 py-2 text-sm font-semibold text-accent-foreground transition focus:translate-y-0"
      >
        跳到主要内容
      </a>

      <header className="sticky top-0 z-50 border-b border-separator bg-background/90 backdrop-blur-xl">
        <div className="mx-auto flex min-h-16 max-w-7xl flex-wrap items-center gap-x-6 gap-y-2 px-4 py-2 sm:px-6 lg:px-8">
          <a href="#top" aria-label="PrismArk 首页" className="shrink-0">
            <PrismArkBrand compact />
          </a>

          <nav className="order-3 flex w-full gap-1 overflow-x-auto pb-1 sm:order-none sm:ml-auto sm:w-auto sm:pb-0" aria-label="主导航">
            {navigation.map((item) => (
              <a
                className="shrink-0 rounded-md px-3 py-2 text-xs font-medium text-muted transition hover:bg-default-soft hover:text-foreground"
                href={item.href}
                key={item.href}
              >
                {item.label}
              </a>
            ))}
          </nav>

          <ThemeToggle showLabels={false} />

          <a
            href={consoleHref}
            className="ml-auto inline-flex h-9 shrink-0 items-center gap-1.5 rounded-md bg-accent px-3.5 text-xs font-semibold text-accent-foreground hover:bg-accent-hover sm:ml-0"
          >
            打开控制台
            <ArrowRight className="size-3.5" aria-hidden="true" />
          </a>
        </div>
      </header>

      <main id="main-content">
        <section id="top" aria-labelledby="hero-title" className="relative scroll-mt-24 overflow-hidden border-b border-separator">
          <div className="pointer-events-none absolute inset-0" aria-hidden="true">
            <div className="absolute left-[8%] top-10 size-72 rounded-full bg-accent/10 blur-3xl" />
            <div className="absolute right-[6%] top-32 size-80 rounded-full bg-cyan-400/10 blur-3xl" />
            <div className="absolute inset-0 opacity-40 [background-image:linear-gradient(var(--separator)_1px,transparent_1px),linear-gradient(90deg,var(--separator)_1px,transparent_1px)] [background-size:64px_64px] [mask-image:linear-gradient(to_bottom,black,transparent_78%)]" />
          </div>

          <div className="relative mx-auto grid max-w-7xl items-center gap-12 px-4 py-20 sm:px-6 sm:py-28 lg:grid-cols-[minmax(0,1fr)_minmax(420px,.92fr)] lg:px-8 lg:py-32">
            <div>
              <Chip size="sm" variant="soft" color="accent">
                <Chip.Label>对象存储 × 内容预览</Chip.Label>
              </Chip>
              <h1 id="hero-title" className="mt-6 max-w-3xl text-4xl font-bold leading-[1.08] text-foreground sm:text-5xl lg:text-6xl">
                存下每一个对象，
                <span className="block bg-gradient-to-r from-accent via-violet-500 to-cyan-500 bg-clip-text text-transparent">
                  看懂每一种格式
                </span>
              </h1>
              <p className="mt-6 max-w-2xl text-lg leading-8 text-muted">
                PrismArk（万象仓）是一套面向团队与开发者的自托管对象存储和内容体验平台：以 S3
                作为核心协议面，以 WebDAV 连接桌面工作流，并把全格式预览与图片 Variant 放进同一条内容链路。
              </p>
              <div className="mt-8 flex flex-col gap-3 sm:flex-row">
                <PrimaryLink href={consoleHref}>
                  开始使用 PrismArk
                  <ArrowRight className="size-4" aria-hidden="true" />
                </PrimaryLink>
                <SecondaryLink href={docsHref}>
                  <Code2 className="size-4" aria-hidden="true" />
                  查看部署方式
                </SecondaryLink>
              </div>
              <ul className="mt-8 grid max-w-xl gap-3 text-sm text-muted sm:grid-cols-3" aria-label="产品特征">
                {['自托管部署', '开放协议优先', '预览体验优先'].map((item) => (
                  <li className="flex items-center gap-2" key={item}>
                    <span className="grid size-5 place-items-center rounded-full bg-success-soft text-success-soft-foreground">
                      <Check className="size-3" aria-hidden="true" />
                    </span>
                    {item}
                  </li>
                ))}
              </ul>
            </div>

            <div className="relative lg:pl-4">
              <div className="absolute -inset-8 rounded-full bg-accent/10 blur-3xl" aria-hidden="true" />
              <PreviewMockup />
            </div>
          </div>
        </section>

        <section id="capabilities" className="scroll-mt-24 py-20 sm:py-28" aria-labelledby="capabilities-title">
          <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
            <div id="capabilities-title">
              <SectionHeading
                eyebrow="Preview first"
                title="对象不应该只是一个下载链接"
                description="PrismArk 把预览放在对象存储的第一现场。文档、图片、代码、表格、数据库与归档文件，都能在浏览器中快速判断内容，再决定下载、分享或继续处理。"
              />
            </div>

            <div className="mt-12 grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
              {previewFormats.map(({ icon, label, detail }) => (
                <FeatureCard icon={icon} title={label} key={label}>
                  {detail}
                </FeatureCard>
              ))}
            </div>

            <div className="mt-16 grid items-center gap-10 lg:grid-cols-2">
              <PreviewMockup />
              <div>
                <p className="text-xs font-bold uppercase text-accent-soft-foreground">Universal preview</p>
                <h3 className="mt-3 text-2xl font-bold text-foreground sm:text-3xl">先看内容，再做动作</h3>
                <p className="mt-4 text-base leading-7 text-muted">
                  统一预览面板保留文件名、MIME、大小和原文件入口，并支持重新加载与全屏查看。无需在多个桌面软件之间来回切换，也不会把预览能力绑定到单一文件类型。
                </p>
                <ul className="mt-6 space-y-3 text-sm text-muted">
                  {[
                    '同一入口承载多种格式，降低内容确认成本',
                    '预览失败时仍保留安全的原文件访问路径',
                    '主题、工具栏和全屏体验保持一致',
                  ].map((item) => (
                    <li className="flex items-start gap-3" key={item}>
                      <Check className="mt-0.5 size-4 shrink-0 text-success" aria-hidden="true" />
                      <span>{item}</span>
                    </li>
                  ))}
                </ul>
              </div>
            </div>
          </div>
        </section>

        <section id="file-experience" className="scroll-mt-24 border-y border-separator bg-background-secondary py-20 sm:py-28" aria-labelledby="explorer-title">
          <div className="mx-auto grid max-w-7xl items-center gap-12 px-4 sm:px-6 lg:grid-cols-[.85fr_1.15fr] lg:px-8">
            <div id="explorer-title">
              <SectionHeading
                eyebrow="File experience"
                title="像文件管理器一样自然，像对象存储一样可靠"
                description="用文件夹、瀑布流卡片和右键菜单组织对象，同时保留适合批量核对的表格视图。它熟悉，但没有把对象模型伪装成传统磁盘。"
              />
              <div className="mt-8 grid gap-4 sm:grid-cols-2 lg:grid-cols-1 xl:grid-cols-2">
                {[
                  [MousePointer2, '直接操作', '双击预览、键盘选择、右键查看详情或执行对象动作。'],
                  [PanelRight, '视图自由', '浏览器视图适合发现内容，表格视图适合批量管理。'],
                  [FolderOpen, '真实前缀', '文件夹来自对象公共前缀，不维护一套虚假的目录副本。'],
                  [Search, '快速定位', '在 Bucket、路径和类型之间筛选，减少深层目录往返。'],
                ].map(([Icon, title, detail]) => {
                  const ItemIcon = Icon as LucideIcon
                  return (
                    <div className="flex gap-3" key={title as string}>
                      <span className="grid size-9 shrink-0 place-items-center rounded-lg bg-accent-soft text-accent-soft-foreground">
                        <ItemIcon className="size-4" aria-hidden="true" />
                      </span>
                      <div>
                        <h3 className="text-sm font-semibold text-foreground">{title as string}</h3>
                        <p className="mt-1 text-xs leading-5 text-muted">{detail as string}</p>
                      </div>
                    </div>
                  )
                })}
              </div>
            </div>
            <ExplorerMockup />
          </div>
        </section>

        <section className="py-20 sm:py-28" aria-labelledby="variant-title">
          <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
            <div className="grid items-center gap-12 lg:grid-cols-2">
              <div className="relative min-h-[430px] overflow-hidden rounded-3xl border border-border bg-[#0b1020] p-6 sm:p-10">
                <div className="absolute inset-0 bg-[radial-gradient(circle_at_30%_20%,rgba(99,102,241,.28),transparent_40%),radial-gradient(circle_at_80%_80%,rgba(34,211,238,.2),transparent_42%)]" />
                <div className="relative flex items-center justify-between text-xs text-white/60">
                  <span>Original</span>
                  <code>2400 × 1600 · PNG</code>
                </div>
                <div className="relative mt-5 grid grid-cols-[1fr_.72fr] items-end gap-4">
                  <div className="aspect-[4/3] rounded-2xl border border-white/15 bg-gradient-to-br from-violet-500 via-indigo-500 to-cyan-400 p-5 shadow-2xl">
                    <PrismArkMark className="size-full text-white" />
                  </div>
                  <div className="mb-5 aspect-square rounded-2xl border border-cyan-300/30 bg-gradient-to-br from-indigo-500 to-cyan-400 p-4 shadow-2xl">
                    <PrismArkMark className="size-full text-white" />
                  </div>
                </div>
                <div className="relative mt-7 rounded-xl border border-white/10 bg-white/5 p-4 text-white">
                  <div className="flex items-center gap-2 text-xs font-semibold">
                    <WandSparkles className="size-4 text-cyan-300" aria-hidden="true" />
                    按请求生成图片 Variant
                  </div>
                  <code className="mt-3 block overflow-x-auto whitespace-nowrap text-[10px] text-white/55">
                    ?width=640&amp;height=640&amp;fit=cover&amp;format=webp
                  </code>
                </div>
              </div>

              <div id="variant-title">
                <SectionHeading
                  eyebrow="Image Variant"
                  title="原图只存一份，交付可以千变万化"
                  description="围绕宽高、裁切、质量与输出格式组合图片 Variant。开发者通过稳定参数获得目标图像，内容团队则能在控制台中直接预览结果。"
                />
                <div className="mt-8 grid gap-4 sm:grid-cols-2">
                  <FeatureCard icon={Braces} title="参数化接口">用明确参数描述输出，不为每个尺寸维护重复文件。</FeatureCard>
                  <FeatureCard icon={Layers3} title="原图与衍生图分离">保留不可变原始对象，把 Variant 视为可重建的派生结果。</FeatureCard>
                </div>
              </div>
            </div>
          </div>
        </section>

        <section id="architecture" className="scroll-mt-24 border-y border-separator bg-background-secondary py-20 sm:py-28" aria-labelledby="architecture-title">
          <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
            <div id="architecture-title">
              <SectionHeading
                eyebrow="Protocol & architecture"
                title="S3 是主协议，WebDAV 是兼容层"
                description="围绕统一对象语义组织认证、权限、Bucket、对象和错误响应。S3 面向应用与云原生工具链，WebDAV 则承接操作系统和桌面软件中的传统文件工作流。"
                align="center"
              />
            </div>

            <div className="mt-12 grid gap-3 lg:grid-cols-[1fr_auto_1.15fr_auto_1fr] lg:items-center">
              <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-1">
                <ArchitectureNode icon={Cloud} title="S3 客户端" detail="SDK、CLI、备份工具与数据管道" />
                <ArchitectureNode icon={HardDrive} title="WebDAV 客户端" detail="系统挂载、办公软件与桌面工作流" />
              </div>
              <ChevronRight className="mx-auto hidden size-5 text-muted lg:block" aria-hidden="true" />
              <div className="rounded-2xl border border-accent/30 bg-accent-soft p-5 text-center shadow-sm">
                <PrismArkMark className="mx-auto size-14" />
                <p className="mt-3 text-base font-bold text-foreground">PrismArk Core</p>
                <div className="mt-4 grid grid-cols-2 gap-2 text-[11px] text-muted">
                  {['认证与签名', '权限策略', '对象语义', '错误映射'].map((item) => (
                    <span className="rounded-md border border-border bg-surface px-2 py-2" key={item}>{item}</span>
                  ))}
                </div>
              </div>
              <ChevronRight className="mx-auto hidden size-5 text-muted lg:block" aria-hidden="true" />
              <div className="grid gap-3 sm:grid-cols-3 lg:grid-cols-1">
                <ArchitectureNode icon={Database} title="元数据" detail="Bucket、对象版本与策略状态" />
                <ArchitectureNode icon={Box} title="对象后端" detail="原始内容与派生结果" />
                <ArchitectureNode icon={Sparkles} title="内容能力" detail="预览、图片 Variant 与后续 AI" />
              </div>
            </div>

            <div id="deployment" className="mt-12 grid gap-4 md:grid-cols-3 scroll-mt-24">
              <FeatureCard icon={PackageOpen} title="单机起步">适合个人、工作室和开发环境，从一个可控部署单元开始。</FeatureCard>
              <FeatureCard icon={Server} title="服务边界清晰">协议、元数据、对象后端与耗时处理保持可演进的模块边界。</FeatureCard>
              <FeatureCard icon={ShieldCheck} title="数据留在你的环境">选择自己的运行环境、网络边界和对象存储后端。</FeatureCard>
            </div>
          </div>
        </section>

        <section id="roadmap" className="scroll-mt-24 py-20 sm:py-28" aria-labelledby="roadmap-title">
          <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
            <div id="roadmap-title">
              <SectionHeading
                eyebrow="Roadmap"
                title="从看见内容，走向理解内容"
                description="AI 能力会沿用现有对象、预览和 Variant 边界逐步接入。路线图只表达产品方向，不把尚未交付的能力包装成现成功能。"
                align="center"
              />
            </div>

            <ol className="mx-auto mt-12 grid max-w-5xl gap-4 md:grid-cols-3">
              {[
                {
                  icon: Eye,
                  phase: '现在',
                  title: '存储与内容体验',
                  detail: '对象管理、通用预览、文件浏览器和图片 Variant 构成统一工作流。',
                  color: 'success' as const,
                },
                {
                  icon: GitBranch,
                  phase: '正在对齐',
                  title: 'S3 协议完整度',
                  detail: '持续补齐协议处理、策略、版本控制、生命周期和兼容性测试。',
                  color: 'warning' as const,
                },
                {
                  icon: Bot,
                  phase: '路线方向',
                  title: 'AI 内容理解',
                  detail: '围绕语义检索、自动标签、摘要与内容工作流接入可替换的 AI 能力。',
                  color: 'accent' as const,
                },
              ].map(({ icon: Icon, phase, title, detail, color }) => (
                <li className="relative rounded-2xl border border-border bg-surface p-6 shadow-sm" key={phase}>
                  <div className="flex items-center justify-between gap-3">
                    <span className="grid size-10 place-items-center rounded-xl bg-default-soft text-foreground">
                      <Icon className="size-5" aria-hidden="true" />
                    </span>
                    <Chip size="sm" variant="soft" color={color}><Chip.Label>{phase}</Chip.Label></Chip>
                  </div>
                  <h3 className="mt-5 text-lg font-semibold text-foreground">{title}</h3>
                  <p className="mt-2 text-sm leading-6 text-muted">{detail}</p>
                </li>
              ))}
            </ol>
          </div>
        </section>

        <section id="get-started" className="scroll-mt-24 px-4 pb-20 sm:px-6 sm:pb-28 lg:px-8" aria-labelledby="cta-title">
          <div className="relative mx-auto max-w-7xl overflow-hidden rounded-3xl border border-accent/30 bg-[#0b1020] px-6 py-14 text-white shadow-[var(--overlay-shadow)] sm:px-12 sm:py-16">
            <div className="absolute inset-0 bg-[radial-gradient(circle_at_10%_10%,rgba(99,102,241,.38),transparent_38%),radial-gradient(circle_at_90%_90%,rgba(34,211,238,.22),transparent_42%)]" aria-hidden="true" />
            <div className="relative grid items-center gap-8 lg:grid-cols-[1fr_auto]">
              <div>
                <div className="flex items-center gap-3 text-cyan-300">
                  <PrismArkMark className="size-10" />
                  <span className="text-xs font-bold uppercase">PrismArk · 万象仓</span>
                </div>
                <h2 id="cta-title" className="mt-5 max-w-3xl text-3xl font-bold text-white sm:text-4xl">让对象存储成为内容工作的起点</h2>
                <p className="mt-4 max-w-2xl text-base leading-7 text-white/65">
                  从自托管部署开始，用开放协议保存对象，用统一预览理解内容，再按你的节奏接入 Variant 与 AI 工作流。
                </p>
              </div>
              <div className="flex flex-col gap-3 sm:flex-row lg:flex-col">
                <a href={consoleHref} className="inline-flex min-h-11 items-center justify-center gap-2 rounded-lg bg-white px-5 py-2.5 text-sm font-semibold text-[#111827] transition hover:bg-white/90">
                  打开控制台
                  <ArrowRight className="size-4" aria-hidden="true" />
                </a>
                <a href={effectiveSourceHref} className="inline-flex min-h-11 items-center justify-center gap-2 rounded-lg border border-white/20 bg-white/5 px-5 py-2.5 text-sm font-semibold text-white transition hover:bg-white/10">
                  <TerminalSquare className="size-4" aria-hidden="true" />
                  获取部署说明
                </a>
              </div>
            </div>
          </div>
        </section>
      </main>

      <footer className="border-t border-separator bg-background-secondary">
        <div className="mx-auto grid max-w-7xl gap-8 px-4 py-10 sm:px-6 md:grid-cols-[1fr_auto] md:items-end lg:px-8">
          <div>
            <PrismArkBrand />
            <p className="mt-4 max-w-lg text-sm leading-6 text-muted">
              面向团队与开发者的自托管对象存储、全格式文件预览与图片 Variant 平台。
            </p>
          </div>
          <nav className="flex flex-wrap gap-x-5 gap-y-3 text-xs text-muted" aria-label="页脚导航">
            <a className="hover:text-foreground" href="#capabilities">产品能力</a>
            <a className="hover:text-foreground" href="#architecture">协议架构</a>
            <a className="hover:text-foreground" href={docsHref}>部署说明</a>
            <a className="hover:text-foreground" href="#roadmap">产品路线</a>
          </nav>
        </div>
        <div className="border-t border-separator">
          <div className="mx-auto flex max-w-7xl flex-col gap-2 px-4 py-5 text-[11px] text-muted sm:flex-row sm:items-center sm:justify-between sm:px-6 lg:px-8">
            <p>PrismArk · Store every object. See every facet.</p>
            <p>S3-first · WebDAV-compatible · Preview-native</p>
          </div>
        </div>
      </footer>
    </div>
  )
}

export default PrismArkLandingPage
