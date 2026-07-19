import { Button, Input } from '@heroui/react'
import {
  ChevronDown,
  ChevronRight,
  Download,
  File,
  FileArchive,
  Folder,
  KeyRound,
  LoaderCircle,
  RefreshCw,
} from 'lucide-react'
import { Fragment, useEffect, useMemo, useRef, useState } from 'react'

import {
  ARCHIVE_MAX_PASSWORD_LENGTH,
  type ArchiveExportRequest,
  type ArchiveScanRequest,
  type ArchiveScanResult,
  type ArchiveWorkerResponse,
} from './archive-protocol'
import { ARCHIVE_PREVIEW_MAX_SOURCE_BYTES } from './ArchivePreviewPolicy'
import { archiveFolderPaths, buildArchiveTree, type ArchiveTreeNode } from './ArchiveTree'
import { formatPreviewLimit, ViewerLoading, ViewerNotice, type ViewerFileProps } from './ViewerShared'

export const ARCHIVE_SCAN_TIMEOUT_MS = 30_000
export const ARCHIVE_EXPORT_TIMEOUT_MS = 120_000

type ArchivePreviewState =
  | { status: 'loading' }
  | { status: 'password'; message: string; result?: ArchiveScanResult }
  | { status: 'success'; result: ArchiveScanResult }
  | { status: 'error'; message: string }

type ArchiveExportState =
  | { status: 'idle' }
  | { status: 'exporting'; path: string }
  | { status: 'error'; message: string }

type ScanAttempt = {
  revision: number
  password: string
}

export default function ArchivePreviewPlugin({ fileName, size, url }: ViewerFileProps) {
  const [scanAttempt, setScanAttempt] = useState<ScanAttempt>({ revision: 0, password: '' })
  const [passwordInput, setPasswordInput] = useState('')
  const [state, setState] = useState<ArchivePreviewState>({ status: 'loading' })
  const [exportState, setExportState] = useState<ArchiveExportState>({ status: 'idle' })
  const passwordRef = useRef('')
  const exportWorkerRef = useRef<Worker | null>(null)
  const exportTimeoutRef = useRef<number | undefined>(undefined)
  const exceedsLimit = size > ARCHIVE_PREVIEW_MAX_SOURCE_BYTES

  useEffect(() => {
    if (exceedsLimit) return

    let worker: Worker | null = null
    let timeoutId: number | undefined
    let settled = false
    setState({ status: 'loading' })
    setExportState({ status: 'idle' })

    const terminate = () => {
      if (!worker) return
      worker.terminate()
      worker = null
    }
    const finish = () => {
      if (timeoutId !== undefined) window.clearTimeout(timeoutId)
      timeoutId = undefined
      terminate()
    }
    const fail = (message: string) => {
      if (settled) return
      settled = true
      finish()
      passwordRef.current = ''
      setState({ status: 'error', message })
    }

    try {
      worker = new Worker(new URL('./archive.worker.ts', import.meta.url), { type: 'module' })
      worker.onmessage = (event: MessageEvent<ArchiveWorkerResponse>) => {
        if (settled) return
        const response = event.data
        if (response?.type === 'scan-success') {
          settled = true
          finish()
          passwordRef.current = scanAttempt.password
          setPasswordInput('')
          setState({ status: 'success', result: response.result })
          return
        }
        if (response?.type === 'password-required') {
          settled = true
          finish()
          passwordRef.current = ''
          setPasswordInput('')
          const visibleResult = response.result && response.result.entries.length > 0 ? response.result : undefined
          setState({ status: 'password', message: response.error, ...(visibleResult ? { result: visibleResult } : {}) })
          return
        }
        if (response?.type === 'error' && response.operation === 'scan') {
          fail(response.error || '压缩包目录读取失败。')
          return
        }
        fail('压缩包查看器返回了无法识别的响应。')
      }
      worker.onerror = (event) => fail(event.message || '压缩包目录读取失败。')
      worker.onmessageerror = () => fail('压缩包查看器返回的数据无法读取。')
      timeoutId = window.setTimeout(() => fail('压缩包目录读取超时，请重试。'), ARCHIVE_SCAN_TIMEOUT_MS)
      const request: ArchiveScanRequest = {
        type: 'scan',
        url,
        sourceSize: size,
        ...(scanAttempt.password ? { password: scanAttempt.password } : {}),
      }
      worker.postMessage(request)
    } catch (cause) {
      fail(cause instanceof Error ? cause.message : '无法启动压缩包查看器。')
    }

    return () => {
      settled = true
      finish()
    }
  }, [exceedsLimit, scanAttempt, size, url])

  useEffect(() => () => {
    exportWorkerRef.current?.terminate()
    if (exportTimeoutRef.current !== undefined) window.clearTimeout(exportTimeoutRef.current)
  }, [])

  const result = state.status === 'success' || state.status === 'password' ? state.result : undefined

  const unlock = () => {
    if (!passwordInput || passwordInput.length > ARCHIVE_MAX_PASSWORD_LENGTH) return
    setScanAttempt((current) => ({ revision: current.revision + 1, password: passwordInput }))
  }

  const retry = () => {
    passwordRef.current = ''
    setPasswordInput('')
    setScanAttempt((current) => ({ revision: current.revision + 1, password: '' }))
  }

  const exportEntry = (path: string, target: 'file' | 'folder') => {
    if (!result || exportState.status === 'exporting' || state.status === 'password') return
    exportWorkerRef.current?.terminate()
    if (exportTimeoutRef.current !== undefined) window.clearTimeout(exportTimeoutRef.current)
    setExportState({ status: 'exporting', path })

    let worker: Worker | null = null
    let settled = false
    const finish = () => {
      if (exportTimeoutRef.current !== undefined) window.clearTimeout(exportTimeoutRef.current)
      exportTimeoutRef.current = undefined
      worker?.terminate()
      if (exportWorkerRef.current === worker) exportWorkerRef.current = null
      worker = null
    }
    const fail = (message: string) => {
      if (settled) return
      settled = true
      finish()
      setExportState({ status: 'error', message })
    }

    try {
      worker = new Worker(new URL('./archive.worker.ts', import.meta.url), { type: 'module' })
      exportWorkerRef.current = worker
      worker.onmessage = (event: MessageEvent<ArchiveWorkerResponse>) => {
        if (settled) return
        const response = event.data
        if (response?.type === 'export-success' && response.path === path) {
          settled = true
          finish()
          downloadArchiveData(response.data, response.fileName, response.mimeType)
          setExportState({ status: 'idle' })
          return
        }
        if (response?.type === 'password-required') {
          settled = true
          finish()
          passwordRef.current = ''
          setExportState({ status: 'idle' })
          setState({ status: 'password', message: response.error, result })
          return
        }
        if (response?.type === 'error' && response.operation === 'export') {
          fail(response.error || '压缩包条目导出失败。')
          return
        }
        fail('压缩包导出器返回了无法识别的响应。')
      }
      worker.onerror = (event) => fail(event.message || '压缩包条目导出失败。')
      worker.onmessageerror = () => fail('压缩包导出器返回的数据无法读取。')
      exportTimeoutRef.current = window.setTimeout(() => fail('压缩包条目导出超时，请重试。'), ARCHIVE_EXPORT_TIMEOUT_MS)
      const request: ArchiveExportRequest = {
        type: 'export',
        url,
        sourceSize: size,
        path,
        target,
        ...(passwordRef.current ? { password: passwordRef.current } : {}),
      }
      worker.postMessage(request)
    } catch (cause) {
      fail(cause instanceof Error ? cause.message : '无法启动压缩包导出器。')
    }
  }

  if (exceedsLimit) {
    return <ViewerNotice title="压缩包超过在线预览上限" description={`压缩包目录预览最多处理 ${formatPreviewLimit(ARCHIVE_PREVIEW_MAX_SOURCE_BYTES)}，请在新窗口打开或下载后查看。`} />
  }
  if (state.status === 'loading') return <ViewerLoading label="正在读取压缩包目录" />
  if (state.status === 'error') return <ArchiveError message={state.message} onRetry={retry} />
  if (state.status === 'password' && !state.result) {
    return <ArchivePasswordOnly fileName={fileName} message={state.message} password={passwordInput} onPasswordChange={setPasswordInput} onUnlock={unlock} />
  }
  if (!result) return <ViewerNotice title="压缩包目录不可用" description="没有获得可显示的目录数据。" />
  if (result.entries.length === 0 && state.status !== 'password') {
    return <ViewerNotice title="压缩包中没有可显示的目录项" description="压缩包可能为空，或使用了暂不支持的目录结构。" />
  }
  return <ArchiveDirectory
    fileName={fileName}
    result={result}
    locked={state.status === 'password'}
    passwordMessage={state.status === 'password' ? state.message : undefined}
    password={passwordInput}
    exportState={exportState}
    onPasswordChange={setPasswordInput}
    onUnlock={unlock}
    onExport={exportEntry}
  />
}

function ArchiveError({ message, onRetry }: { message: string; onRetry: () => void }) {
  return <div className="flex h-full min-h-0 w-full flex-col bg-[#111317]">
    <div className="min-h-0 flex-1"><ViewerNotice title="压缩包目录读取失败" description={message} /></div>
    <div className="flex shrink-0 justify-center border-t border-white/10 bg-[#191c21] px-4 py-3"><Button variant="secondary" size="sm" onClick={onRetry}><RefreshCw className="size-4" />重试</Button></div>
  </div>
}

function ArchivePasswordOnly({
  fileName,
  message,
  password,
  onPasswordChange,
  onUnlock,
}: {
  fileName: string
  message: string
  password: string
  onPasswordChange: (value: string) => void
  onUnlock: () => void
}) {
  return <div data-testid="archive-preview" data-viewer-plugin="archive" className="flex h-full min-h-0 w-full flex-col bg-white text-[#1f2937]">
    <ArchiveHeader fileName={fileName} />
    <div className="grid min-h-0 flex-1 place-items-center bg-[#f8fafc] p-6">
      <div className="w-full max-w-sm rounded-lg border border-[#d9dee7] bg-white p-5 shadow-sm">
        <span className="mx-auto grid size-10 place-items-center rounded-md bg-[#fff7ed] text-[#c2410c]"><KeyRound className="size-5" /></span>
        <p className="mt-3 text-center text-sm font-semibold">此压缩包已加密</p>
        <PasswordForm message={message} password={password} onPasswordChange={onPasswordChange} onUnlock={onUnlock} centered />
      </div>
    </div>
  </div>
}

function ArchiveDirectory({
  fileName,
  result,
  locked,
  passwordMessage,
  password,
  exportState,
  onPasswordChange,
  onUnlock,
  onExport,
}: {
  fileName: string
  result: ArchiveScanResult
  locked: boolean
  passwordMessage?: string
  password: string
  exportState: ArchiveExportState
  onPasswordChange: (value: string) => void
  onUnlock: () => void
  onExport: (path: string, target: 'file' | 'folder') => void
}) {
  const tree = useMemo(() => buildArchiveTree(result.entries), [result.entries])
  const folderPaths = useMemo(() => archiveFolderPaths(tree), [tree])
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set(folderPaths))
  const truncated = result.truncated || result.entries.length < result.entryCount
  const truncationReason = result.truncationReason?.trim() || `仅显示前 ${result.entries.length} 项，共 ${result.entryCount} 项。`
  const exportingPath = exportState.status === 'exporting' ? exportState.path : undefined

  useEffect(() => setExpanded(new Set(folderPaths)), [folderPaths])

  const toggleFolder = (path: string) => {
    setExpanded((current) => {
      const next = new Set(current)
      if (next.has(path)) next.delete(path)
      else next.add(path)
      return next
    })
  }

  const rows = (nodes: ArchiveTreeNode[], depth = 0): React.ReactNode => nodes.map((node) => {
    const isExpanded = expanded.has(node.path)
    const folderDownloadDisabled = truncated || node.children.length === 0
    const exportDisabled = locked || exportState.status === 'exporting' || (node.directory && folderDownloadDisabled)
    const downloadLabel = node.directory ? `下载目录 ${node.path}` : `下载文件 ${node.path}`
    return <Fragment key={`${node.directory ? 'directory' : 'file'}:${node.path}`}>
      <li className="grid min-h-11 grid-cols-[minmax(0,1fr)_5.5rem_2.25rem] items-center gap-2 border-b border-[#e5e7eb] px-2 text-xs" style={{ paddingLeft: `${8 + Math.min(depth, 12) * 18}px` }} title={node.path}>
        {node.directory ? <button type="button" className="flex min-w-0 items-center gap-1.5 rounded px-1 py-2 text-left hover:bg-[#f1f5f9]" aria-label={`${isExpanded ? '收起' : '展开'}目录 ${node.path}`} aria-expanded={isExpanded} onClick={() => toggleFolder(node.path)}>
          {isExpanded ? <ChevronDown className="size-3.5 shrink-0 text-[#64748b]" /> : <ChevronRight className="size-3.5 shrink-0 text-[#64748b]" />}
          <Folder className="size-4 shrink-0 text-[#d97706]" />
          <span className="truncate font-mono text-[#334155]">{node.name}</span>
        </button> : <span className="flex min-w-0 items-center gap-1.5 px-1 py-2">
          <span className="size-3.5 shrink-0" />
          <File className="size-4 shrink-0 text-[#64748b]" />
          <span className="truncate font-mono text-[#334155]">{node.name}</span>
        </span>}
        <span className="shrink-0 text-right tabular-nums text-[#64748b]">{node.directory ? `${countFiles(node)} 个文件` : formatArchiveSize(node.size)}</span>
        <span title={node.directory && truncated ? '目录列表不完整，不能下载整个目录' : downloadLabel}>
          <Button
            isIconOnly
            size="sm"
            variant="ghost"
            aria-label={downloadLabel}
            isDisabled={exportDisabled}
            onClick={() => onExport(node.path, node.directory ? 'folder' : 'file')}
          >
            {exportingPath === node.path ? <LoaderCircle className="size-4 animate-spin" /> : <Download className="size-4" />}
          </Button>
        </span>
      </li>
      {node.directory && isExpanded ? rows(node.children, depth + 1) : null}
    </Fragment>
  })

  return <div data-testid="archive-preview" data-viewer-plugin="archive" className="flex h-full min-h-0 w-full flex-col bg-white text-[#1f2937]">
    <ArchiveHeader fileName={fileName} result={result} />
    {locked && <PasswordForm message={passwordMessage || '此压缩包需要密码。'} password={password} onPasswordChange={onPasswordChange} onUnlock={onUnlock} />}
    <ul aria-label="压缩包目录" className="min-h-0 flex-1 overflow-auto">{rows(tree)}</ul>
    {exportState.status === 'exporting' && <p className="shrink-0 border-t border-[#bfdbfe] bg-[#eff6ff] px-3 py-2 text-[11px] text-[#1d4ed8]">正在导出 {exportState.path}</p>}
    {exportState.status === 'error' && <p role="alert" className="shrink-0 border-t border-[#fecaca] bg-[#fef2f2] px-3 py-2 text-[11px] text-[#b91c1c]">{exportState.message}</p>}
    {truncated && <p className="shrink-0 border-t border-[#e6d7a8] bg-[#fffbea] px-3 py-2 text-[11px] leading-5 text-[#854d0e]">目录已截断：{truncationReason}</p>}
  </div>
}

function ArchiveHeader({ fileName, result }: { fileName: string; result?: ArchiveScanResult }) {
  return <div className="flex min-h-11 shrink-0 items-center gap-2 border-b border-[#d9dee7] bg-[#f3f5f8] px-3 text-xs" title={fileName}>
    <FileArchive className="size-4 shrink-0 text-[#2563eb]" />
    <span className="truncate font-semibold text-[#334155]">压缩包目录</span>
    {result && <span className="ml-auto shrink-0 text-right tabular-nums text-[#64748b]">{result.entryCount} 项 · 声明大小 {formatArchiveSize(result.totalDeclaredSize)}</span>}
  </div>
}

function PasswordForm({
  message,
  password,
  centered = false,
  onPasswordChange,
  onUnlock,
}: {
  message: string
  password: string
  centered?: boolean
  onPasswordChange: (value: string) => void
  onUnlock: () => void
}) {
  return <form data-testid="archive-password-form" className={centered ? 'mt-4 space-y-3' : 'flex shrink-0 flex-col gap-2 border-b border-[#fed7aa] bg-[#fff7ed] px-3 py-2 sm:flex-row sm:items-center'} onSubmit={(event) => { event.preventDefault(); onUnlock() }}>
    {!centered && <KeyRound className="hidden size-4 shrink-0 text-[#c2410c] sm:block" />}
    <Input fullWidth aria-label="压缩包密码" type="password" autoComplete="off" maxLength={ARCHIVE_MAX_PASSWORD_LENGTH} placeholder="输入压缩包密码" value={password} onChange={(event) => onPasswordChange(event.target.value)} />
    <Button type="submit" variant="primary" size="sm" className={centered ? 'w-full' : 'shrink-0'} isDisabled={!password}>解锁</Button>
    <p role="status" className={centered ? 'text-center text-xs text-[#9a3412]' : 'text-[11px] text-[#9a3412] sm:max-w-48'}>{message}</p>
  </form>
}

function downloadArchiveData(data: ArrayBuffer, fileName: string, mimeType: string): void {
  const objectUrl = URL.createObjectURL(new Blob([data], { type: mimeType }))
  const link = document.createElement('a')
  link.href = objectUrl
  link.download = fileName
  link.hidden = true
  document.body.append(link)
  link.click()
  link.remove()
  window.setTimeout(() => URL.revokeObjectURL(objectUrl), 0)
}

function countFiles(node: ArchiveTreeNode): number {
  return node.children.reduce((total, child) => total + (child.directory ? countFiles(child) : 1), 0)
}

function formatArchiveSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '--'
  if (bytes < 1024) return `${Math.round(bytes)} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let value = bytes / 1024
  let unitIndex = 0
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024
    unitIndex += 1
  }
  const precision = value >= 10 ? 0 : 1
  return `${value.toFixed(precision).replace(/\.0$/, '')} ${units[unitIndex]}`
}
