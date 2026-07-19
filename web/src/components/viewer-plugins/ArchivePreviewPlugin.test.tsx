import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import ArchivePreviewPlugin, { ARCHIVE_SCAN_TIMEOUT_MS } from './ArchivePreviewPlugin'
import { ARCHIVE_PREVIEW_MAX_SOURCE_BYTES } from './ArchivePreviewPolicy'

class MockWorker {
  static instances: MockWorker[] = []
  onerror: ((event: ErrorEvent) => unknown) | null = null
  onmessage: ((event: MessageEvent) => unknown) | null = null
  onmessageerror: ((event: MessageEvent) => unknown) | null = null
  postMessage = vi.fn()
  terminate = vi.fn()

  constructor(public url: URL, public options?: WorkerOptions) {
    MockWorker.instances.push(this)
  }

  emit(data: unknown) {
    this.onmessage?.({ data } as MessageEvent)
  }
}

const scanResult = {
  entries: [
    { path: 'docs', size: 0, directory: true },
    { path: 'docs/readme.txt', size: 1536, directory: false },
    { path: 'docs/guides/start.md', size: 2048, directory: false },
  ],
  entryCount: 3,
  totalDeclaredSize: 3584,
  truncated: false,
  encrypted: false,
}

const originalWorker = globalThis.Worker
const originalCreateObjectUrl = Object.getOwnPropertyDescriptor(URL, 'createObjectURL')
const originalRevokeObjectUrl = Object.getOwnPropertyDescriptor(URL, 'revokeObjectURL')
let downloaded: Array<{ href: string; name: string }>

beforeEach(() => {
  MockWorker.instances = []
  downloaded = []
  Object.defineProperty(globalThis, 'Worker', { configurable: true, writable: true, value: MockWorker })
  Object.defineProperty(URL, 'createObjectURL', { configurable: true, value: vi.fn(() => 'blob:archive-export') })
  Object.defineProperty(URL, 'revokeObjectURL', { configurable: true, value: vi.fn() })
  vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function downloadClick(this: HTMLAnchorElement) {
    downloaded.push({ href: this.href, name: this.download })
  })
})

afterEach(() => {
  cleanup()
  vi.useRealTimers()
  vi.restoreAllMocks()
  Object.defineProperty(globalThis, 'Worker', { configurable: true, writable: true, value: originalWorker })
  restoreUrlMethod('createObjectURL', originalCreateObjectUrl)
  restoreUrlMethod('revokeObjectURL', originalRevokeObjectUrl)
})

describe('ArchivePreviewPlugin', () => {
  it('renders an expanded directory tree with file and folder download actions', async () => {
    render(<ArchivePreviewPlugin fileName="release.zip" mimeType="application/zip" size={4096} url="/signed/release.zip" />)
    const worker = MockWorker.instances[0]
    expect(worker.options).toEqual({ type: 'module' })
    expect(worker.postMessage).toHaveBeenCalledWith({ type: 'scan', url: '/signed/release.zip', sourceSize: 4096 })

    act(() => worker.emit({ type: 'scan-success', result: scanResult }))

    expect(await screen.findByTestId('archive-preview')).toHaveAttribute('data-viewer-plugin', 'archive')
    expect(screen.getByRole('button', { name: '收起目录 docs' })).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByText('readme.txt')).toBeInTheDocument()
    expect(screen.getByText('start.md')).toBeInTheDocument()
    expect(screen.getByText('1.5 KB')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '下载目录 docs' })).toBeEnabled()
    expect(screen.getByRole('button', { name: '下载文件 docs/readme.txt' })).toBeEnabled()
    expect(worker.terminate).toHaveBeenCalledOnce()
  })

  it('exports one file directly and one folder as a generated ZIP', async () => {
    render(<ArchivePreviewPlugin fileName="release.zip" mimeType="application/zip" size={4096} url="/signed/release.zip" />)
    act(() => MockWorker.instances[0].emit({ type: 'scan-success', result: scanResult }))

    fireEvent.click(await screen.findByRole('button', { name: '下载文件 docs/readme.txt' }))
    const fileWorker = MockWorker.instances[1]
    expect(fileWorker.postMessage).toHaveBeenCalledWith({
      type: 'export',
      url: '/signed/release.zip',
      sourceSize: 4096,
      path: 'docs/readme.txt',
      target: 'file',
    })
    act(() => fileWorker.emit({
      type: 'export-success',
      path: 'docs/readme.txt',
      fileName: 'readme.txt',
      mimeType: 'application/octet-stream',
      data: new Uint8Array([1, 2, 3]).buffer,
    }))
    expect(downloaded).toEqual([{ href: 'blob:archive-export', name: 'readme.txt' }])
    expect(fileWorker.terminate).toHaveBeenCalledOnce()

    fireEvent.click(screen.getByRole('button', { name: '下载目录 docs' }))
    const folderWorker = MockWorker.instances[2]
    expect(folderWorker.postMessage).toHaveBeenCalledWith({
      type: 'export',
      url: '/signed/release.zip',
      sourceSize: 4096,
      path: 'docs',
      target: 'folder',
    })
    act(() => folderWorker.emit({
      type: 'export-success',
      path: 'docs',
      fileName: 'docs.zip',
      mimeType: 'application/zip',
      data: new Uint8Array([80, 75]).buffer,
    }))
    expect(downloaded[1]).toEqual({ href: 'blob:archive-export', name: 'docs.zip' })
    expect(folderWorker.terminate).toHaveBeenCalledOnce()
  })

  it('unlocks an encrypted archive and asks again after an invalid export password', async () => {
    render(<ArchivePreviewPlugin fileName="secret.7z" mimeType="application/x-7z-compressed" size={4096} url="/signed/secret.7z" />)
    const encryptedResult = { ...scanResult, encrypted: true }
    act(() => MockWorker.instances[0].emit({
      type: 'password-required',
      invalid: false,
      error: '此压缩包已加密，需要密码。',
      result: encryptedResult,
    }))

    expect(await screen.findByTestId('archive-password-form')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '下载文件 docs/readme.txt' })).toBeDisabled()
    fireEvent.change(screen.getByLabelText('压缩包密码'), { target: { value: 'secret' } })
    fireEvent.click(screen.getByRole('button', { name: '解锁' }))

    const unlockWorker = MockWorker.instances[1]
    expect(unlockWorker.postMessage).toHaveBeenCalledWith({
      type: 'scan',
      url: '/signed/secret.7z',
      sourceSize: 4096,
      password: 'secret',
    })
    act(() => unlockWorker.emit({ type: 'scan-success', result: encryptedResult }))
    const downloadButton = await screen.findByRole('button', { name: '下载文件 docs/readme.txt' })
    expect(downloadButton).toBeEnabled()
    fireEvent.click(downloadButton)

    const exportWorker = MockWorker.instances[2]
    expect(exportWorker.postMessage).toHaveBeenCalledWith(expect.objectContaining({ type: 'export', password: 'secret' }))
    act(() => exportWorker.emit({ type: 'password-required', invalid: true, error: '密码不正确，请重新输入。' }))
    expect(await screen.findByText('密码不正确，请重新输入。')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '下载文件 docs/readme.txt' })).toBeDisabled()
  })

  it('keeps the scanned tree locked and clears an invalid scan password', async () => {
    render(<ArchivePreviewPlugin fileName="secret.7z" mimeType="application/x-7z-compressed" size={4096} url="/signed/secret.7z" />)
    const encryptedResult = { ...scanResult, encrypted: true }
    act(() => MockWorker.instances[0].emit({
      type: 'password-required',
      invalid: false,
      error: '此压缩包已加密，需要密码。',
      result: encryptedResult,
    }))

    const password = await screen.findByLabelText('压缩包密码')
    fireEvent.change(password, { target: { value: 'wrong-password' } })
    fireEvent.click(screen.getByRole('button', { name: '解锁' }))
    act(() => MockWorker.instances[1].emit({
      type: 'password-required',
      invalid: true,
      error: '密码不正确，请重新输入。',
      result: encryptedResult,
    }))

    expect(await screen.findByText('密码不正确，请重新输入。')).toBeInTheDocument()
    expect(screen.getByLabelText('压缩包密码')).toHaveValue('')
    expect(screen.getByText('readme.txt')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '下载文件 docs/readme.txt' })).toBeDisabled()
  })

  it('shows a focused password prompt when encrypted headers hide every entry', async () => {
    render(<ArchivePreviewPlugin fileName="secret-header.7z" mimeType="application/x-7z-compressed" size={4096} url="/signed/secret-header.7z" />)
    act(() => MockWorker.instances[0].emit({
      type: 'password-required',
      invalid: false,
      error: '此压缩包已加密，需要密码。',
      result: { ...scanResult, entries: [], entryCount: 0, totalDeclaredSize: 0, encrypted: true },
    }))

    expect(await screen.findByTestId('archive-password-form')).toBeInTheDocument()
    expect(screen.queryByRole('list', { name: '压缩包目录' })).not.toBeInTheDocument()
    expect(screen.getByText('此压缩包已加密')).toBeInTheDocument()
  })

  it('terminates a failed Worker and creates a fresh Worker when retrying', async () => {
    render(<ArchivePreviewPlugin fileName="broken.zip" mimeType="application/zip" size={1024} url="/signed/broken.zip" />)
    const firstWorker = MockWorker.instances[0]
    act(() => firstWorker.emit({ type: 'error', operation: 'scan', error: '中央目录损坏' }))

    expect(await screen.findByText('压缩包目录读取失败')).toBeInTheDocument()
    expect(screen.getByText('中央目录损坏')).toBeInTheDocument()
    expect(firstWorker.terminate).toHaveBeenCalledOnce()
    fireEvent.click(screen.getByRole('button', { name: '重试' }))

    await waitFor(() => expect(MockWorker.instances).toHaveLength(2))
    expect(MockWorker.instances[1].postMessage).toHaveBeenCalledWith({ type: 'scan', url: '/signed/broken.zip', sourceSize: 1024 })
  })

  it('times out, cleans up active Workers, and rejects sources above the limit', () => {
    vi.useFakeTimers()
    const slowView = render(<ArchivePreviewPlugin fileName="slow.zip" mimeType="application/zip" size={1024} url="/signed/slow.zip" />)
    const worker = MockWorker.instances[0]

    act(() => vi.advanceTimersByTime(ARCHIVE_SCAN_TIMEOUT_MS))
    expect(screen.getByText('压缩包目录读取超时，请重试。')).toBeInTheDocument()
    expect(worker.terminate).toHaveBeenCalledOnce()
    slowView.unmount()

    const activeView = render(<ArchivePreviewPlugin fileName="active.zip" mimeType="application/zip" size={1024} url="/signed/active.zip" />)
    const activeWorker = MockWorker.instances[1]
    activeView.unmount()
    expect(activeWorker.terminate).toHaveBeenCalledOnce()

    render(<ArchivePreviewPlugin fileName="large.zip" mimeType="application/zip" size={ARCHIVE_PREVIEW_MAX_SOURCE_BYTES + 1} url="/signed/large.zip" />)
    expect(screen.getByText('压缩包超过在线预览上限')).toBeInTheDocument()
    expect(MockWorker.instances).toHaveLength(2)
  })
})

function restoreUrlMethod(name: 'createObjectURL' | 'revokeObjectURL', descriptor: PropertyDescriptor | undefined) {
  if (descriptor) Object.defineProperty(URL, name, descriptor)
  else Reflect.deleteProperty(URL, name)
}
