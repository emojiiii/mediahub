import {
  assetPlugin,
  audioPlugin,
  cadPlugin,
  drawingPlugin,
  emailPlugin,
  epubPlugin,
  gisPlugin,
  imagePlugin,
  model3dPlugin,
  ofdPlugin,
  officePlugin,
  pdfPlugin,
  textPlugin,
  videoPlugin,
  xpsPlugin,
  type PreviewPlugin,
  type PreviewSource,
} from '@open-file-viewer/core'
import '@open-file-viewer/core/style.css'
import { FileViewer } from '@open-file-viewer/react'
import { Button } from '@heroui/react'
import { Download, RefreshCw, X } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import pdfWorkerSrc from 'pdfjs-dist/build/pdf.worker.mjs?url'

import { createMediaHubArchivePlugin, isArchiveFile } from './viewer-plugins/OpenFileViewerArchivePlugin'
import { createMediaHubSpreadsheetPlugin, isSpreadsheetFile } from './viewer-plugins/OpenFileViewerSpreadsheetPlugin'
import { createMediaHubSqlitePlugin, isSqliteFile } from './viewer-plugins/OpenFileViewerSqlitePlugin'
import {
  downloadBufferedPreviewFile,
  formatDownloadBytes,
  isDownloadAbortError,
  type BufferedPreviewProgress,
} from './viewer-plugins/BufferedPreviewDownload'
import { BUFFERED_PREVIEW_MAX_BYTES } from './viewer-plugins/BufferedPreviewPolicy'
import { formatPreviewLimit, ViewerNotice, type ViewerFileProps } from './viewer-plugins/ViewerShared'

export const TEXT_PREVIEW_MAX_BYTES = BUFFERED_PREVIEW_MAX_BYTES
export const IMAGE_PREVIEW_MAX_BYTES = BUFFERED_PREVIEW_MAX_BYTES
export const PDF_PREVIEW_MAX_BYTES = BUFFERED_PREVIEW_MAX_BYTES
export const GENERAL_PREVIEW_MAX_BYTES = BUFFERED_PREVIEW_MAX_BYTES

export type ViewerAdmissionKind =
  | 'archive'
  | 'audio-video'
  | 'image'
  | 'pdf'
  | 'spreadsheet'
  | 'sqlite'
  | 'text'
  | 'general'

const audioVideoExtensions = new Set([
  'aac', 'avi', 'flac', 'flv', 'm2ts', 'm3u8', 'm4a', 'm4v', 'mid', 'midi', 'mkv', 'mov', 'mp3', 'mp4',
  'mpeg', 'mpg', 'oga', 'ogg', 'ogv', 'opus', 'wav', 'webm', 'wma', 'wmv',
])
const imageExtensions = new Set([
  'apng', 'avif', 'bmp', 'cur', 'gif', 'heic', 'heif', 'ico', 'jfif', 'jpeg', 'jpg', 'jxl', 'pjpe', 'pjpeg',
  'png', 'svg', 'tif', 'tiff', 'webp',
])
const textExtensions = new Set([
  'astro', 'bash', 'bat', 'bib', 'c', 'cjs', 'clj', 'cljs', 'cmd', 'conf', 'config', 'cpp', 'cs', 'css',
  'cts', 'dart', 'diff', 'dockerfile', 'editorconfig', 'elm', 'env', 'erl', 'ex', 'exs', 'fish', 'fs', 'fsx',
  'gitignore', 'go', 'gql', 'graphql', 'gradle', 'h', 'hcl', 'hpp', 'hrl', 'hs', 'htm', 'html', 'http',
  'ini', 'ipynb', 'java', 'js', 'json', 'json5', 'jsonc', 'jsonl', 'jsx', 'kt', 'kts', 'latex', 'less',
  'lhs', 'lock', 'log', 'lua', 'md', 'mjs', 'mts', 'ndjson', 'nginxconf', 'npmrc', 'patch', 'php', 'proto',
  'properties', 'ps1', 'py', 'r', 'rb', 'rs', 'scss', 'sh', 'sql', 'svelte', 'svg', 'swift', 'tex', 'tf',
  'tfvars', 'toml', 'ts', 'tsv', 'tsx', 'txt', 'vue', 'xml', 'yaml', 'yml', 'zsh',
])

function extensionOf(fileName: string): string {
  const leaf = fileName.trim().toLowerCase().split(/[\\/]/).pop() ?? ''
  if (!leaf.includes('.')) return leaf
  return leaf.split('.').pop() ?? ''
}

export function detectViewerAdmissionKind(fileName: string, mimeType: string): ViewerAdmissionKind {
  const extension = extensionOf(fileName)
  const mime = mimeType.toLowerCase().split(';', 1)[0].trim()

  if (isArchiveFile({ extension, mimeType: mime })) return 'archive'
  if (isSqliteFile({ extension, mimeType: mime })) return 'sqlite'
  if (isSpreadsheetFile({ extension, mimeType: mime })) return 'spreadsheet'
  if (mime.startsWith('audio/') || mime.startsWith('video/') || audioVideoExtensions.has(extension)) return 'audio-video'
  if (mime === 'application/pdf' || extension === 'pdf') return 'pdf'
  if (mime.startsWith('image/') || imageExtensions.has(extension)) {
    return extension === 'svg' || mime === 'image/svg+xml' ? 'text' : 'image'
  }
  if (
    mime.startsWith('text/')
    || mime.includes('json')
    || mime.includes('javascript')
    || mime.includes('typescript')
    || mime === 'application/xml'
    || mime.endsWith('+xml')
    || textExtensions.has(extension)
  ) return 'text'
  return 'general'
}

export function previewLimitForFile(fileName: string, mimeType: string): number | null {
  const kind = detectViewerAdmissionKind(fileName, mimeType)
  if (kind === 'audio-video') return null
  return BUFFERED_PREVIEW_MAX_BYTES
}

export function normalizeViewerMimeType(fileName: string, mimeType: string): string {
  const extension = extensionOf(fileName)
  const mime = mimeType.toLowerCase().split(';', 1)[0].trim()
  return extension === 'svg' || mime === 'image/svg+xml' ? 'text/xml' : mimeType
}

export function createViewerPlugins(sourceSize: number): PreviewPlugin[] {
  const assetBase = `${import.meta.env.BASE_URL.replace(/\/?$/, '/')}pdfjs/`
  return [
    createMediaHubArchivePlugin(sourceSize),
    createMediaHubSqlitePlugin(sourceSize),
    createMediaHubSpreadsheetPlugin(sourceSize),
    officePlugin(),
    textPlugin(),
    imagePlugin(),
    videoPlugin(),
    audioPlugin(),
    pdfPlugin({
      workerSrc: pdfWorkerSrc,
      cMapUrl: `${assetBase}cmaps/`,
      cMapPacked: true,
      standardFontDataUrl: `${assetBase}standard_fonts/`,
      useSystemFonts: true,
    }),
    epubPlugin(),
    xpsPlugin(),
    ofdPlugin(),
    emailPlugin(),
    drawingPlugin(),
    cadPlugin(),
    model3dPlugin(),
    gisPlugin(),
    assetPlugin(),
  ]
}

export default function ObjectFileViewer({ fileName, mimeType, size, url }: ViewerFileProps) {
  const previewLimit = previewLimitForFile(fileName, mimeType)

  if (previewLimit != null && size > previewLimit) {
    return <ViewerNotice
      title="文件超过在线预览上限"
      description={`该格式最多在线处理 ${formatPreviewLimit(previewLimit)}，请在新窗口打开或下载后查看。`}
    />
  }

  if (previewLimit !== null) {
    return <BufferedObjectFileViewer
      key={`${url}\u0000${fileName}\u0000${mimeType}`}
      fileName={fileName}
      mimeType={mimeType}
      size={size}
      url={url}
    />
  }

  return <OpenFileViewerSurface fileName={fileName} mimeType={mimeType} source={url} sourceSize={size} />
}

type BufferedDownloadState =
  | { status: 'downloading'; progress: BufferedPreviewProgress }
  | { status: 'ready'; file: File }
  | { status: 'error'; message: string }
  | { status: 'cancelled' }

function BufferedObjectFileViewer({ fileName, mimeType, size, url }: ViewerFileProps) {
  const [attempt, setAttempt] = useState(0)
  const [state, setState] = useState<BufferedDownloadState>({
    status: 'downloading',
    progress: { loadedBytes: 0, totalBytes: null },
  })
  const controllerRef = useRef<AbortController | null>(null)

  useEffect(() => {
    const controller = new AbortController()
    let active = true
    controllerRef.current = controller
    setState({ status: 'downloading', progress: { loadedBytes: 0, totalBytes: null } })
    void downloadBufferedPreviewFile({
      url,
      fileName,
      mimeType,
      signal: controller.signal,
      maxBytes: BUFFERED_PREVIEW_MAX_BYTES,
      onProgress: (progress) => {
        if (active) setState({ status: 'downloading', progress })
      },
    }).then((file) => {
      if (active) setState({ status: 'ready', file })
    }).catch((cause) => {
      if (!active || (isDownloadAbortError(cause) && controller.signal.aborted)) return
      setState({ status: 'error', message: cause instanceof Error ? cause.message : '预览文件下载失败' })
    })
    return () => {
      active = false
      if (controllerRef.current === controller) controllerRef.current = null
      controller.abort()
    }
  }, [attempt, fileName, mimeType, url])

  if (state.status === 'ready') {
    return <OpenFileViewerSurface fileName={fileName} mimeType={mimeType} source={state.file} sourceSize={state.file.size} />
  }
  if (state.status === 'downloading') {
    return <BufferedDownloadProgress
      fileName={fileName}
      progress={state.progress}
      onCancel={() => {
        controllerRef.current?.abort()
        setState({ status: 'cancelled' })
      }}
    />
  }
  if (state.status === 'cancelled') {
    return <DownloadFailure
      title="预览下载已取消"
      description="已停止读取该文件。"
      onRetry={() => setAttempt((value) => value + 1)}
    />
  }
  return <DownloadFailure
    title="预览文件下载失败"
    description={state.message}
    onRetry={() => setAttempt((value) => value + 1)}
  />
}

function BufferedDownloadProgress({ fileName, progress, onCancel }: { fileName: string; progress: BufferedPreviewProgress; onCancel: () => void }) {
  const percentage = progress.totalBytes === null
    ? null
    : progress.totalBytes === 0
      ? 100
      : Math.min(100, Math.round((progress.loadedBytes / progress.totalBytes) * 100))
  const valueLabel = percentage === null
    ? `${formatDownloadBytes(progress.loadedBytes)} · 总大小未知`
    : `${formatDownloadBytes(progress.loadedBytes)} / ${formatDownloadBytes(progress.totalBytes!)} · ${percentage}%`

  return <div data-testid="buffered-preview-download" className="flex h-full min-h-0 flex-col bg-[#111317] text-white">
    <div className="grid min-h-0 flex-1 place-items-center px-6">
      <div className="w-full max-w-md">
        <span className="mx-auto grid size-12 place-items-center rounded-lg border border-white/10 bg-white/[.06] text-white/75"><Download className="size-5" /></span>
        <h3 className="mt-4 truncate text-center text-sm font-semibold" title={fileName}>正在下载预览文件</h3>
        <div
          role="progressbar"
          aria-label="预览文件下载进度"
          aria-valuetext={valueLabel}
          {...(percentage === null ? {} : { 'aria-valuemin': 0, 'aria-valuemax': 100, 'aria-valuenow': percentage })}
          className="mt-5 h-1.5 overflow-hidden rounded-full bg-white/10"
        >
          <span className={percentage === null ? 'block h-full w-1/3 animate-pulse rounded-full bg-[#4f8cff]' : 'block h-full rounded-full bg-[#4f8cff] transition-[width]'} style={percentage === null ? undefined : { width: `${percentage}%` }} />
        </div>
        <p aria-live="polite" className="mt-2 text-center text-xs tabular-nums text-white/55">{valueLabel}</p>
      </div>
    </div>
    <div className="flex shrink-0 justify-center border-t border-white/10 bg-[#191c21] px-4 py-3">
      <Button variant="secondary" size="sm" onClick={onCancel}><X className="size-4" />取消</Button>
    </div>
  </div>
}

function DownloadFailure({ title, description, onRetry }: { title: string; description: string; onRetry: () => void }) {
  return <div className="flex h-full min-h-0 flex-col bg-[#111317]">
    <div className="min-h-0 flex-1"><ViewerNotice title={title} description={description} /></div>
    <div className="flex shrink-0 justify-center border-t border-white/10 bg-[#191c21] px-4 py-3"><Button variant="secondary" size="sm" onClick={onRetry}><RefreshCw className="size-4" />重试</Button></div>
  </div>
}

function OpenFileViewerSurface({ fileName, mimeType, source, sourceSize }: { fileName: string; mimeType: string; source: PreviewSource; sourceSize: number }) {
  const plugins = useMemo(() => createViewerPlugins(sourceSize), [sourceSize])
  return <div data-testid="open-file-viewer" className="h-full min-h-0 w-full min-w-0 overflow-hidden bg-white">
    <FileViewer
      file={source}
      fileName={fileName}
      mimeType={normalizeViewerMimeType(fileName, mimeType)}
      width="100%"
      height="100%"
      style={{ width: '100%', height: '100%', minWidth: 0, minHeight: 0 }}
      fit="contain"
      locale="zh-CN"
      theme="light"
      toolbar={false}
      plugins={plugins}
    />
  </div>
}
