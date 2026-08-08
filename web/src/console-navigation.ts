import {
  Boxes,
  Database,
  Eye,
  ImageIcon,
  KeyRound,
  Layers3,
  LayoutDashboard,
  LockKeyhole,
  Settings,
  ShieldCheck,
  Tags,
  Webhook,
  type LucideIcon,
} from 'lucide-react'

export type ConsoleNavItem = {
  label: string
  path: string
  icon: LucideIcon
  state?: 'planned'
}

export type ConsoleNavGroup = {
  label: string
  items: ConsoleNavItem[]
}

export const consoleNavGroups: ConsoleNavGroup[] = [
  {
    label: '工作区',
    items: [
      { label: '总览', path: 'dashboard', icon: LayoutDashboard },
      { label: '文件浏览器', path: 'objects', icon: Boxes },
      { label: 'Buckets', path: 'buckets', icon: Database },
    ],
  },
  {
    label: '数据保护',
    items: [
      { label: 'Policies', path: 'policies', icon: ShieldCheck, state: 'planned' },
      { label: 'Versioning', path: 'versioning', icon: Layers3, state: 'planned' },
      { label: 'Lifecycle', path: 'lifecycle', icon: Tags, state: 'planned' },
      { label: 'Object Lock', path: 'object-lock', icon: LockKeyhole, state: 'planned' },
    ],
  },
  {
    label: '内容体验',
    items: [
      { label: '预览中心', path: 'previews', icon: Eye },
      { label: '图片 Variant', path: 'variants', icon: ImageIcon },
    ],
  },
  {
    label: '访问与事件',
    items: [
      { label: '访问密钥', path: 'access-keys', icon: KeyRound },
      { label: 'Webhooks', path: 'webhooks', icon: Webhook },
    ],
  },
]

export const settingsNavItem: ConsoleNavItem = { label: '设置', path: 'settings', icon: Settings }

export function consoleNavItem(path: string): ConsoleNavItem | undefined {
  return [...consoleNavGroups.flatMap((group) => group.items), settingsNavItem].find((item) => item.path === path)
}
