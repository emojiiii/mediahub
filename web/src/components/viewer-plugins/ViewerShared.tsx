import { Spinner } from '@heroui/react'
import { FileText } from 'lucide-react'
import type { Root } from 'react-dom/client'

export type ViewerFileProps = {
  fileName: string
  mimeType: string
  size: number
  url: string
}

export function deferViewerRootCleanup(root: Pick<Root, 'unmount'>, mount: HTMLElement, cleanup?: () => void): void {
  queueMicrotask(() => {
    try {
      root.unmount()
    } finally {
      mount.remove()
      cleanup?.()
    }
  })
}

export function formatPreviewLimit(bytes: number): string {
  return bytes >= 1024 * 1024 ? `${Math.round(bytes / (1024 * 1024))} MB` : `${Math.round(bytes / 1024)} KB`
}

export function ViewerLoading({ label = '正在生成预览' }: { label?: string }) {
  return <div className="grid h-full place-items-center bg-[#111317] text-white/70"><div className="flex items-center gap-3 text-sm"><Spinner aria-label={label} color="accent" size="sm" />{label}</div></div>
}

export function ViewerNotice({ title, description }: { title: string; description: string }) {
  return <div className="grid h-full place-items-center bg-[#111317] px-6 text-center text-white"><div className="max-w-md"><span className="mx-auto grid size-12 place-items-center rounded-lg border border-white/10 bg-white/[.06] text-white/75"><FileText className="size-5" /></span><h3 className="mt-4 text-sm font-semibold text-white">{title}</h3><p className="mt-2 text-xs leading-5 text-white/55">{description}</p></div></div>
}

export function decodeSource(buffer: ArrayBuffer): string {
  return new TextDecoder('utf-8', { fatal: false }).decode(buffer).replace(/^\uFEFF/, '')
}
