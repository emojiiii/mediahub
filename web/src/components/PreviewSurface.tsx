import { Button } from '@heroui/react'
import { Download, Expand, File, Minimize2, RefreshCw } from 'lucide-react'
import { useEffect, useState } from 'react'

import type { ResolvedTheme } from '../theme'
import ObjectFileViewer from './ObjectFileViewer'

export interface PreviewSurfaceProps {
  fileName: string
  mimeType: string
  size: number
  url: string
  theme: ResolvedTheme
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '大小未知'
  if (bytes === 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  const unit = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1)
  const value = bytes / 1024 ** unit
  return `${value >= 10 || unit === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[unit]}`
}

export function PreviewSurface({ fileName, mimeType, size, url, theme }: PreviewSurfaceProps) {
  const [reloadKey, setReloadKey] = useState(0)
  const [fullscreen, setFullscreen] = useState(false)

  useEffect(() => {
    if (!fullscreen) return
    const previousOverflow = document.body.style.overflow
    const exitOnEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setFullscreen(false)
    }
    document.body.style.overflow = 'hidden'
    document.addEventListener('keydown', exitOnEscape)
    return () => {
      document.body.style.overflow = previousOverflow
      document.removeEventListener('keydown', exitOnEscape)
    }
  }, [fullscreen])

  return (
    <section
      aria-label={`预览 ${fileName}`}
      data-testid="preview-surface"
      className={`${fullscreen ? 'fixed inset-0 z-[100]' : 'h-full min-h-0 w-full'} flex min-w-0 flex-col overflow-hidden bg-surface text-foreground`}
    >
      <header className="flex min-h-12 shrink-0 items-center gap-3 border-b border-separator bg-surface/95 px-3 backdrop-blur sm:px-4">
        <span className="grid size-8 shrink-0 place-items-center rounded-md bg-accent-soft text-accent-soft-foreground"><File className="size-4" /></span>
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-xs font-semibold text-foreground" title={fileName}>{fileName}</h3>
          <p className="mt-0.5 truncate text-[10px] text-muted">{mimeType || 'application/octet-stream'} · {formatBytes(size)}</p>
        </div>
        <div className="flex shrink-0 items-center gap-1" role="group" aria-label="预览操作">
          <Button isIconOnly size="sm" variant="ghost" aria-label="重新加载预览" onClick={() => setReloadKey((value) => value + 1)}><RefreshCw className="size-4" /></Button>
          <a className="button button--sm button--ghost grid size-8 place-items-center p-0" href={url} target="_blank" rel="noopener noreferrer" aria-label="下载原文件" title="下载原文件"><Download className="size-4" /></a>
          <Button isIconOnly size="sm" variant="ghost" aria-label={fullscreen ? '退出全屏预览' : '全屏预览'} onClick={() => setFullscreen((value) => !value)}>{fullscreen ? <Minimize2 className="size-4" /> : <Expand className="size-4" />}</Button>
        </div>
      </header>
      <div className="min-h-0 flex-1 overflow-hidden bg-background-secondary">
        <ObjectFileViewer key={`${url}\u0000${reloadKey}`} fileName={fileName} mimeType={mimeType} size={size} url={url} theme={theme} />
      </div>
    </section>
  )
}
