import { useQuery } from '@tanstack/react-query'
import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent as ReactMouseEvent } from 'react'
import {
  Archive,
  Braces,
  Check,
  File,
  FileAudio,
  FileImage,
  FileSpreadsheet,
  FileText,
  FileVideo2,
  Folder,
  Info,
  LoaderCircle,
  Pencil,
  Play,
  Presentation,
  Trash2,
} from 'lucide-react'

import { api, type ObjectItem, type VariantParams } from '../api'

export type FileKind =
  | 'image'
  | 'video'
  | 'audio'
  | 'pdf'
  | 'document'
  | 'spreadsheet'
  | 'presentation'
  | 'archive'
  | 'code'
  | 'text'
  | 'other'

export interface FileExplorerProps {
  items: ObjectItem[]
  prefixes: string[]
  currentPrefix: string
  selectedIds: string[]
  deletingId?: string | null
  onOpenFolder: (prefix: string) => void
  onSelectionChange: (ids: string[]) => void
  onPreview: (item: ObjectItem) => void
  onOpenDetails: (item: ObjectItem) => void
  onEdit: (item: ObjectItem) => void
  onDelete: (item: ObjectItem) => void
}

type ContextMenuState =
  | { kind: 'folder'; prefix: string; name: string; x: number; y: number }
  | { kind: 'file'; item: ObjectItem; x: number; y: number }

type KindPresentation = {
  label: string
  icon: typeof File
  color: string
  preview: string
}

const KIND_PRESENTATION: Record<FileKind, KindPresentation> = {
  image: { label: '图片', icon: FileImage, color: 'text-fuchsia-700 dark:text-fuchsia-300', preview: 'bg-gradient-to-br from-fuchsia-100 via-pink-50 to-violet-100 dark:from-fuchsia-950/70 dark:via-pink-950/40 dark:to-violet-950/70' },
  video: { label: '视频', icon: FileVideo2, color: 'text-violet-700 dark:text-violet-300', preview: 'bg-gradient-to-br from-violet-100 via-indigo-50 to-slate-100 dark:from-violet-950/70 dark:via-indigo-950/50 dark:to-slate-900' },
  audio: { label: '音频', icon: FileAudio, color: 'text-emerald-700 dark:text-emerald-300', preview: 'bg-gradient-to-br from-emerald-100 via-teal-50 to-cyan-100 dark:from-emerald-950/70 dark:via-teal-950/50 dark:to-cyan-950/70' },
  pdf: { label: 'PDF', icon: FileText, color: 'text-red-700 dark:text-red-300', preview: 'bg-gradient-to-br from-red-100 via-rose-50 to-orange-50 dark:from-red-950/70 dark:via-rose-950/40 dark:to-orange-950/50' },
  document: { label: '文档', icon: FileText, color: 'text-blue-700 dark:text-blue-300', preview: 'bg-gradient-to-br from-blue-100 via-sky-50 to-white dark:from-blue-950/70 dark:via-sky-950/40 dark:to-slate-900' },
  spreadsheet: { label: '表格', icon: FileSpreadsheet, color: 'text-green-700 dark:text-green-300', preview: 'bg-gradient-to-br from-green-100 via-emerald-50 to-white dark:from-green-950/70 dark:via-emerald-950/40 dark:to-slate-900' },
  presentation: { label: '演示文稿', icon: Presentation, color: 'text-orange-700 dark:text-orange-300', preview: 'bg-gradient-to-br from-orange-100 via-amber-50 to-white dark:from-orange-950/70 dark:via-amber-950/40 dark:to-slate-900' },
  archive: { label: '压缩包', icon: Archive, color: 'text-amber-700 dark:text-amber-300', preview: 'bg-gradient-to-br from-amber-100 via-yellow-50 to-stone-100 dark:from-amber-950/70 dark:via-yellow-950/30 dark:to-stone-900' },
  code: { label: '代码', icon: Braces, color: 'text-cyan-700 dark:text-cyan-300', preview: 'bg-gradient-to-br from-cyan-100 via-slate-50 to-blue-100 dark:from-cyan-950/70 dark:via-slate-900 dark:to-blue-950/70' },
  text: { label: '文本', icon: FileText, color: 'text-slate-700 dark:text-slate-300', preview: 'bg-gradient-to-br from-slate-100 via-zinc-50 to-white dark:from-slate-800 dark:via-zinc-900 dark:to-slate-950' },
  other: { label: '文件', icon: File, color: 'text-zinc-700 dark:text-zinc-300', preview: 'bg-gradient-to-br from-zinc-100 via-neutral-50 to-white dark:from-zinc-800 dark:via-neutral-900 dark:to-slate-950' },
}

/** Returns the direct child folder label represented by an S3 common prefix. */
export function folderName(prefix: string, currentPrefix = ''): string {
  const normalizedPrefix = prefix.replace(/^\/+|\/+$/g, '')
  const normalizedCurrent = currentPrefix.replace(/^\/+|\/+$/g, '')
  const relative = normalizedCurrent && normalizedPrefix.startsWith(`${normalizedCurrent}/`)
    ? normalizedPrefix.slice(normalizedCurrent.length + 1)
    : normalizedPrefix
  const segments = relative.split('/').filter(Boolean)
  const fallbackSegments = normalizedPrefix.split('/').filter(Boolean)
  return segments[0] ?? fallbackSegments[fallbackSegments.length - 1] ?? prefix
}

/** Classifies a MIME value without relying on file extensions or remote data. */
export function classifyMimeType(mimeType: string): FileKind {
  const mime = mimeType.split(';', 1)[0]?.trim().toLowerCase() ?? ''
  if (mime.startsWith('image/')) return 'image'
  if (mime.startsWith('video/')) return 'video'
  if (mime.startsWith('audio/')) return 'audio'
  if (mime === 'application/pdf') return 'pdf'
  if (/spreadsheet|excel|csv/.test(mime)) return 'spreadsheet'
  if (/presentation|powerpoint/.test(mime)) return 'presentation'
  if (/wordprocessing|msword|opendocument\.text|epub/.test(mime)) return 'document'
  if (/zip|compressed|archive|x-tar|x-rar|x-7z|gzip|bzip/.test(mime)) return 'archive'
  if (
    /(?:json|javascript|typescript|xml|yaml|x-yaml|sql|x-sh|wasm)/.test(mime)
    || /text\/(?:html|css|jsx|tsx|x-python|x-rust|x-go)/.test(mime)
  ) return 'code'
  if (mime.startsWith('text/')) return 'text'
  return 'other'
}

function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return value === 0 ? '0 B' : '—'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  const unit = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1)
  const amount = value / 1024 ** unit
  return `${amount >= 10 || unit === 0 ? amount.toFixed(0) : amount.toFixed(1)} ${units[unit]}`
}

function extension(name: string): string {
  const dot = name.lastIndexOf('.')
  return dot > 0 && dot < name.length - 1 ? name.slice(dot + 1, dot + 6).toUpperCase() : 'FILE'
}

function joinClasses(...classes: Array<string | false | null | undefined>): string {
  return classes.filter(Boolean).join(' ')
}

const MAX_THUMBNAIL_URL_REQUESTS = 4
const THUMBNAIL_PREFETCH_MARGIN = '240px 0px'
const THUMBNAIL_VARIANT_PARAMS: VariantParams = {
  width: 384,
  height: 384,
  fit: 'inside',
  quality: 68,
  format: 'webp',
  blur: 0,
  crop: 'center',
  background: 'ffffff',
}
const VARIANT_IMAGE_MIME_TYPES = new Set([
  'image/bmp',
  'image/gif',
  'image/jpeg',
  'image/png',
  'image/tiff',
  'image/vnd.microsoft.icon',
  'image/webp',
  'image/x-icon',
])
let activeThumbnailUrlRequests = 0
const queuedThumbnailUrlRequests: Array<() => void> = []

function normalizedMimeType(value: string): string {
  return value.trim().toLowerCase().split(';', 1)[0] ?? ''
}

function drainThumbnailUrlQueue() {
  while (activeThumbnailUrlRequests < MAX_THUMBNAIL_URL_REQUESTS) {
    const start = queuedThumbnailUrlRequests.shift()
    if (!start) return
    start()
  }
}

function abortError(): DOMException {
  return new DOMException('Thumbnail URL request was cancelled', 'AbortError')
}

function withThumbnailUrlRequestLimit<T>(request: () => Promise<T>, signal: AbortSignal): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    let started = false
    let settled = false

    const removeAbortListener = () => signal.removeEventListener('abort', cancel)
    const finish = () => {
      activeThumbnailUrlRequests -= 1
      drainThumbnailUrlQueue()
    }
    const start = () => {
      if (settled) return
      started = true
      activeThumbnailUrlRequests += 1
      Promise.resolve()
        .then(request)
        .then((value) => {
          if (settled) return
          settled = true
          removeAbortListener()
          resolve(value)
        }, (error: unknown) => {
          if (settled) return
          settled = true
          removeAbortListener()
          reject(error)
        })
        .finally(finish)
    }
    function cancel() {
      if (settled) return
      settled = true
      removeAbortListener()
      if (!started) {
        const queuedIndex = queuedThumbnailUrlRequests.indexOf(start)
        if (queuedIndex >= 0) queuedThumbnailUrlRequests.splice(queuedIndex, 1)
      }
      reject(abortError())
    }

    if (signal.aborted) {
      cancel()
    } else {
      signal.addEventListener('abort', cancel, { once: true })
      if (activeThumbnailUrlRequests < MAX_THUMBNAIL_URL_REQUESTS) start()
      else queuedThumbnailUrlRequests.push(start)
    }
  })
}

function useNearViewport<T extends HTMLElement>() {
  const ref = useRef<T>(null)
  const [isNearViewport, setIsNearViewport] = useState(false)

  useEffect(() => {
    if (isNearViewport || !ref.current) return
    if (typeof IntersectionObserver === 'undefined') {
      setIsNearViewport(true)
      return
    }

    const node = ref.current
    const scrollRoot = node.closest<HTMLElement>('[data-testid="object-explorer-scroll"]')
    const observer = new IntersectionObserver((entries) => {
      if (!entries.some((entry) => entry.isIntersecting)) return
      setIsNearViewport(true)
      observer.disconnect()
    }, { root: scrollRoot, rootMargin: THUMBNAIL_PREFETCH_MARGIN })
    observer.observe(node)
    return () => observer.disconnect()
  }, [isNearViewport])

  return { ref, isNearViewport }
}

function ImageThumbnail({ item }: { item: ObjectItem }) {
  const { ref, isNearViewport } = useNearViewport<HTMLDivElement>()
  const [loadedUrl, setLoadedUrl] = useState<string | null>(null)
  const [failedUrl, setFailedUrl] = useState<string | null>(null)
  const mimeType = normalizedMimeType(item.type)
  const useVariant = VARIANT_IMAGE_MIME_TYPES.has(mimeType)
  const thumbnailUrl = useQuery({
    queryKey: ['object-thumbnail-url', 'v2', item.id, item.revision, item.visibility, mimeType, useVariant ? THUMBNAIL_VARIANT_PARAMS : 'original'],
    queryFn: async ({ signal }) => {
      const result = await withThumbnailUrlRequestLimit(
        () => useVariant
          ? api.getVariantUrl(item.id, THUMBNAIL_VARIANT_PARAMS)
          : item.visibility === '公开'
            ? api.getPublicUrl(item.id)
            : api.getSignedUrl(item.id),
        signal,
      )
      if (!result.url.trim()) throw new Error('Thumbnail URL is empty')
      return result
    },
    enabled: isNearViewport,
    staleTime: item.visibility === '公开' ? Infinity : 4 * 60 * 1000,
    gcTime: item.visibility === '公开' ? 30 * 60 * 1000 : 10 * 60 * 1000,
    retry: false,
    refetchOnWindowFocus: false,
  })
  const url = isNearViewport ? thumbnailUrl.data?.url : undefined
  const imageLoaded = Boolean(url && loadedUrl === url)
  const imageFailed = thumbnailUrl.isError || Boolean(url && failedUrl === url)

  return (
    <div ref={ref} className="absolute inset-0">
      {!imageLoaded && !imageFailed && (
        <div data-testid={`thumbnail-loading-${item.id}`} aria-hidden="true" className="absolute inset-0 animate-pulse bg-fuchsia-100/55 dark:bg-fuchsia-950/25">
          <div className="absolute inset-x-[18%] bottom-[18%] h-1.5 rounded-full bg-fuchsia-300/25 dark:bg-fuchsia-300/10" />
        </div>
      )}
      {url && !imageFailed && (
        <img
          src={url}
          alt={`${item.name} 缩略图`}
          loading="lazy"
          decoding="async"
          draggable={false}
          referrerPolicy="no-referrer"
          className={joinClasses('absolute inset-0 size-full object-cover transition-opacity duration-200', imageLoaded ? 'opacity-100' : 'opacity-0')}
          onLoad={() => setLoadedUrl(url)}
          onError={() => setFailedUrl(url)}
        />
      )}
      {imageFailed && <FileImage data-testid={`thumbnail-fallback-${item.id}`} aria-hidden="true" className="absolute inset-0 m-auto size-12 text-fuchsia-700 drop-shadow-sm dark:text-fuchsia-300" strokeWidth={1.45} />}
    </div>
  )
}

function FilePreview({ item }: { item: ObjectItem }) {
  const kind = classifyMimeType(item.type)
  const presentation = KIND_PRESENTATION[kind]
  const Icon = presentation.icon
  const height = kind === 'image' ? 'h-40' : kind === 'video' ? 'h-32' : 'h-28'

  return (
    <div className={joinClasses('relative grid overflow-hidden rounded-xl ring-1 ring-inset ring-black/5 dark:ring-white/5', height, presentation.preview)}>
      {kind === 'image' ? <ImageThumbnail item={item} /> : <>
        <div aria-hidden="true" className="absolute inset-0 opacity-25 [background-image:linear-gradient(to_right,currentColor_1px,transparent_1px),linear-gradient(to_bottom,currentColor_1px,transparent_1px)] [background-size:24px_24px]" />
        <Icon aria-hidden="true" className={joinClasses('relative m-auto size-12 drop-shadow-sm', presentation.color)} strokeWidth={1.45} />
      </>}
      {kind === 'video' && <span className="absolute inset-0 m-auto grid size-9 place-items-center rounded-full bg-black/65 text-white shadow-lg"><Play aria-hidden="true" className="ml-0.5 size-4 fill-current" /></span>}
      <span className="absolute bottom-2 left-2 rounded-md bg-white/75 px-1.5 py-0.5 text-[10px] font-semibold tracking-wide text-slate-700 shadow-sm backdrop-blur dark:bg-black/45 dark:text-slate-200">{extension(item.name)}</span>
    </div>
  )
}

export default function FileExplorer({
  items,
  prefixes,
  currentPrefix,
  selectedIds,
  deletingId,
  onOpenFolder,
  onSelectionChange,
  onPreview,
  onOpenDetails,
  onEdit,
  onDelete,
}: FileExplorerProps) {
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null)
  const [activeFolder, setActiveFolder] = useState<string | null>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const selected = new Set(selectedIds)

  const toggleSelection = (id: string) => {
    onSelectionChange(selected.has(id)
      ? selectedIds.filter((selectedId) => selectedId !== id)
      : [...new Set([...selectedIds, id])])
  }

  const selectFromPointer = (id: string, additive: boolean) => {
    setActiveFolder(null)
    if (additive) {
      toggleSelection(id)
    } else if (selectedIds.length !== 1 || selectedIds[0] !== id) {
      onSelectionChange([id])
    }
  }

  const menuPosition = (event: ReactMouseEvent, estimatedHeight: number) => {
    const viewportWidth = typeof window === 'undefined' ? event.clientX + 240 : window.innerWidth
    const viewportHeight = typeof window === 'undefined' ? event.clientY + estimatedHeight : window.innerHeight
    return {
      x: Math.max(8, Math.min(event.clientX, viewportWidth - 232)),
      y: Math.max(8, Math.min(event.clientY, viewportHeight - estimatedHeight - 8)),
    }
  }

  const openFileMenu = (event: ReactMouseEvent, item: ObjectItem) => {
    event.preventDefault()
    const position = menuPosition(event, 250)
    setContextMenu({ kind: 'file', item, ...position })
  }

  const openFolderMenu = (event: ReactMouseEvent, prefix: string) => {
    event.preventDefault()
    const position = menuPosition(event, 64)
    setActiveFolder(prefix)
    setContextMenu({ kind: 'folder', prefix, name: folderName(prefix, currentPrefix), ...position })
  }

  useEffect(() => {
    if (!contextMenu) return
    menuRef.current?.querySelector<HTMLButtonElement>('[role="menuitem"]:not(:disabled)')?.focus()
    const dismissOutside = (event: PointerEvent) => {
      if (!menuRef.current?.contains(event.target as Node)) setContextMenu(null)
    }
    const dismissFromKeyboard = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setContextMenu(null)
    }
    const dismiss = () => setContextMenu(null)
    document.addEventListener('pointerdown', dismissOutside)
    document.addEventListener('keydown', dismissFromKeyboard)
    window.addEventListener('resize', dismiss)
    window.addEventListener('scroll', dismiss, true)
    return () => {
      document.removeEventListener('pointerdown', dismissOutside)
      document.removeEventListener('keydown', dismissFromKeyboard)
      window.removeEventListener('resize', dismiss)
      window.removeEventListener('scroll', dismiss, true)
    }
  }, [contextMenu])

  const runMenuAction = (action: () => void) => {
    setContextMenu(null)
    action()
  }

  const moveMenuFocus = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return
    event.preventDefault()
    const buttons = Array.from(menuRef.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)') ?? [])
    if (!buttons.length) return
    const current = buttons.indexOf(document.activeElement as HTMLButtonElement)
    const next = event.key === 'Home'
      ? 0
      : event.key === 'End'
        ? buttons.length - 1
        : event.key === 'ArrowDown'
          ? (current + 1 + buttons.length) % buttons.length
          : (current - 1 + buttons.length) % buttons.length
    buttons[next]?.focus()
  }

  const menuButton = (label: string, Icon: typeof File, action: () => void, options?: { danger?: boolean; disabled?: boolean }) => (
    <button
      key={label}
      type="button"
      role="menuitem"
      disabled={options?.disabled}
      className={joinClasses(
        'flex min-h-9 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-sm outline-none transition hover:bg-black/[0.055] focus:bg-blue-500/10 focus:text-blue-700 disabled:cursor-not-allowed disabled:opacity-45 dark:hover:bg-white/[0.07] dark:focus:text-blue-300',
        options?.danger && 'text-red-600 dark:text-red-400',
      )}
      onClick={() => runMenuAction(action)}
    >
      <Icon aria-hidden="true" className="size-4" />
      <span>{label}</span>
    </button>
  )

  const empty = prefixes.length === 0 && items.length === 0

  return (
    <section aria-label="对象文件浏览器" className="min-h-0 w-full">
      {empty ? (
        <div className="grid min-h-64 place-items-center rounded-2xl border border-dashed border-slate-300 bg-slate-50/60 px-6 text-center dark:border-slate-700 dark:bg-slate-900/40">
          <div>
            <Folder aria-hidden="true" className="mx-auto size-11 text-slate-400" strokeWidth={1.35} />
            <p className="mt-3 text-sm font-medium text-slate-700 dark:text-slate-200">此文件夹为空</p>
            <p className="mt-1 text-xs text-slate-500 dark:text-slate-400">上传对象后会显示在这里</p>
          </div>
        </div>
      ) : (
        <div role="listbox" aria-label="文件和文件夹" aria-multiselectable="true" className="columns-2 gap-3 sm:columns-3 lg:columns-4 xl:columns-5 2xl:columns-6">
          {prefixes.map((prefix) => {
            const name = folderName(prefix, currentPrefix)
            const isActive = activeFolder === prefix
            return (
              <article key={`folder:${prefix}`} className="mb-3 inline-block w-full break-inside-avoid align-top">
                <div
                  role="option"
                  aria-label={`文件夹 ${name}`}
                  aria-selected={isActive}
                  tabIndex={0}
                  className={joinClasses(
                    'group cursor-default rounded-2xl border bg-white/80 p-2.5 shadow-sm outline-none backdrop-blur transition duration-150 hover:-translate-y-0.5 hover:shadow-md focus-visible:ring-2 focus-visible:ring-blue-500/70 dark:bg-slate-900/80',
                    isActive ? 'border-blue-500 bg-blue-50/90 ring-2 ring-blue-500/20 dark:bg-blue-950/45' : 'border-slate-200/90 dark:border-slate-700/90',
                  )}
                  onClick={() => setActiveFolder(prefix)}
                  onDoubleClick={() => onOpenFolder(prefix)}
                  onContextMenu={(event) => openFolderMenu(event, prefix)}
                  onKeyDown={(event) => {
                    if (event.target !== event.currentTarget) return
                    if (event.key === 'Enter') { event.preventDefault(); onOpenFolder(prefix) }
                    if (event.key === ' ') { event.preventDefault(); setActiveFolder(prefix) }
                  }}
                >
                  <div className="relative grid h-24 place-items-center overflow-hidden rounded-xl bg-gradient-to-br from-amber-100 via-yellow-50 to-orange-100 ring-1 ring-inset ring-amber-200/60 dark:from-amber-950/70 dark:via-yellow-950/35 dark:to-orange-950/60 dark:ring-amber-800/40">
                    <Folder aria-hidden="true" className="size-14 fill-amber-400/70 text-amber-600 drop-shadow-sm dark:fill-amber-500/35 dark:text-amber-300" strokeWidth={1.25} />
                  </div>
                  <div className="px-1 pb-1 pt-2.5">
                    <p className="truncate text-sm font-semibold text-slate-800 dark:text-slate-100" title={name}>{name}</p>
                    <p className="mt-0.5 truncate text-[11px] text-slate-500 dark:text-slate-400" title={prefix}>文件夹 · {prefix}</p>
                  </div>
                </div>
              </article>
            )
          })}

          {items.map((item) => {
            const isSelected = selected.has(item.id)
            const isDeleting = deletingId === item.id
            const kind = classifyMimeType(item.type)
            return (
              <article key={item.id} className="mb-3 inline-block w-full break-inside-avoid align-top">
                <div
                  role="option"
                  aria-label={`文件 ${item.name}`}
                  aria-selected={isSelected}
                  aria-busy={isDeleting || undefined}
                  tabIndex={0}
                  className={joinClasses(
                    'group relative cursor-default rounded-2xl border bg-white/80 p-2.5 shadow-sm outline-none backdrop-blur transition duration-150 hover:-translate-y-0.5 hover:shadow-md focus-visible:ring-2 focus-visible:ring-blue-500/70 dark:bg-slate-900/80',
                    isSelected ? 'border-blue-500 bg-blue-50/90 ring-2 ring-blue-500/20 dark:bg-blue-950/45' : 'border-slate-200/90 dark:border-slate-700/90',
                  )}
                  onClick={(event) => selectFromPointer(item.id, event.ctrlKey || event.metaKey)}
                  onDoubleClick={() => onPreview(item)}
                  onContextMenu={(event) => openFileMenu(event, item)}
                  onKeyDown={(event) => {
                    if (event.target !== event.currentTarget) return
                    if (event.key === 'Enter') { event.preventDefault(); onPreview(item) }
                    if (event.key === ' ') { event.preventDefault(); toggleSelection(item.id) }
                  }}
                >
                  <button
                    type="button"
                    aria-label={isSelected ? `取消选择 ${item.name}` : `选择 ${item.name}`}
                    aria-pressed={isSelected}
                    className={joinClasses(
                      'absolute left-4 top-4 z-10 grid size-6 place-items-center rounded-full border shadow-sm outline-none transition focus-visible:ring-2 focus-visible:ring-blue-500',
                      isSelected ? 'border-blue-600 bg-blue-600 text-white' : 'border-white/90 bg-white/75 text-transparent opacity-0 backdrop-blur group-hover:opacity-100 group-focus-within:opacity-100 dark:border-slate-600 dark:bg-slate-900/75',
                    )}
                    onClick={(event) => { event.stopPropagation(); toggleSelection(item.id) }}
                  >
                    <Check aria-hidden="true" className="size-3.5" strokeWidth={3} />
                  </button>
                  <FilePreview item={item} />
                  <div className="px-1 pb-1 pt-2.5">
                    <p className="truncate text-sm font-semibold text-slate-800 dark:text-slate-100" title={item.name}>{item.name}</p>
                    <div className="mt-1 flex min-w-0 items-center gap-1.5 text-[11px] text-slate-500 dark:text-slate-400">
                      <span className={joinClasses('shrink-0 font-medium', KIND_PRESENTATION[kind].color)}>{KIND_PRESENTATION[kind].label}</span>
                      <span aria-hidden="true">·</span>
                      <span className="truncate tabular-nums">{formatBytes(item.size)}</span>
                    </div>
                  </div>
                  {isDeleting && <div className="absolute inset-0 z-20 grid place-items-center rounded-2xl bg-white/70 backdrop-blur-sm dark:bg-slate-950/70"><span className="flex items-center gap-2 text-xs font-medium text-slate-700 dark:text-slate-200"><LoaderCircle aria-hidden="true" className="size-4 animate-spin" />正在删除</span></div>}
                </div>
              </article>
            )
          })}
        </div>
      )}

      {contextMenu && (
        <div
          ref={menuRef}
          role="menu"
          aria-label={`${contextMenu.kind === 'file' ? contextMenu.item.name : contextMenu.name} 操作菜单`}
          className="fixed z-50 w-56 rounded-xl border border-slate-200/90 bg-white/95 p-1.5 text-slate-700 shadow-2xl shadow-slate-900/15 backdrop-blur-xl dark:border-slate-700 dark:bg-slate-900/95 dark:text-slate-200"
          style={{ left: contextMenu.x, top: contextMenu.y }}
          onContextMenu={(event) => event.preventDefault()}
          onKeyDown={moveMenuFocus}
        >
          {contextMenu.kind === 'folder' ? menuButton('打开', Folder, () => onOpenFolder(contextMenu.prefix)) : <>
            {menuButton('打开 / 预览', Play, () => onPreview(contextMenu.item))}
            {menuButton('查看详情', Info, () => onOpenDetails(contextMenu.item))}
            {menuButton(selected.has(contextMenu.item.id) ? '取消选择' : '选择', Check, () => toggleSelection(contextMenu.item.id))}
            <div role="separator" className="my-1 h-px bg-slate-200 dark:bg-slate-700" />
            {menuButton('编辑', Pencil, () => onEdit(contextMenu.item))}
            {menuButton(isDeletingContext(contextMenu, deletingId) ? '正在删除' : '删除', isDeletingContext(contextMenu, deletingId) ? LoaderCircle : Trash2, () => onDelete(contextMenu.item), { danger: true, disabled: isDeletingContext(contextMenu, deletingId) })}
          </>}
        </div>
      )}
    </section>
  )
}

function isDeletingContext(contextMenu: ContextMenuState, deletingId?: string | null): boolean {
  return contextMenu.kind === 'file' && contextMenu.item.id === deletingId
}
