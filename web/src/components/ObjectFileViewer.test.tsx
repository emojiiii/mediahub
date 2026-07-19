import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const { fileViewerSpy } = vi.hoisted(() => ({ fileViewerSpy: vi.fn() }))

vi.mock('@open-file-viewer/react', async () => {
  const React = await import('react')
  return {
    FileViewer: (props: Record<string, unknown>) => {
      fileViewerSpy(props)
      return React.createElement('div', { 'data-testid': 'upstream-file-viewer' })
    },
  }
})

vi.mock('@open-file-viewer/core', () => {
  const createPlugin = (name: string) => () => ({ name, match: () => false, render: () => ({ destroy() {} }) })
  return {
    assetPlugin: createPlugin('asset'),
    audioPlugin: createPlugin('audio'),
    cadPlugin: createPlugin('cad'),
    drawingPlugin: createPlugin('drawing'),
    emailPlugin: createPlugin('email'),
    epubPlugin: createPlugin('epub'),
    gisPlugin: createPlugin('gis'),
    imagePlugin: createPlugin('image'),
    model3dPlugin: createPlugin('model3d'),
    ofdPlugin: createPlugin('ofd'),
    officePlugin: createPlugin('office'),
    pdfPlugin: createPlugin('pdf'),
    textPlugin: createPlugin('text'),
    videoPlugin: createPlugin('video'),
    xpsPlugin: createPlugin('xps'),
  }
})

import ObjectFileViewer, {
  GENERAL_PREVIEW_MAX_BYTES,
  IMAGE_PREVIEW_MAX_BYTES,
  PDF_PREVIEW_MAX_BYTES,
  TEXT_PREVIEW_MAX_BYTES,
  createViewerPlugins,
  detectViewerAdmissionKind,
  normalizeViewerMimeType,
  previewLimitForFile,
} from './ObjectFileViewer'
import { ARCHIVE_PREVIEW_MAX_SOURCE_BYTES } from './viewer-plugins/ArchivePreviewPolicy'
import { BUFFERED_PREVIEW_MAX_BYTES } from './viewer-plugins/BufferedPreviewPolicy'
import { SPREADSHEET_PREVIEW_MAX_SOURCE_BYTES } from './viewer-plugins/spreadsheet-protocol'
import { SQLITE_PREVIEW_MAX_SOURCE_BYTES } from './viewer-plugins/sqlite-protocol'

beforeEach(() => fileViewerSpy.mockClear())

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

describe('ObjectFileViewer', () => {
  it('builds the upstream plugin set with the bounded archive plugin first', () => {
    const names = createViewerPlugins(4096).map((plugin) => plugin.name)

    expect(names).toEqual([
      'mediahub-archive', 'mediahub-sqlite', 'mediahub-spreadsheet',
      'office', 'text', 'image', 'video', 'audio', 'pdf', 'epub', 'xps', 'ofd',
      'email', 'drawing', 'cad', 'model3d', 'gis', 'asset',
    ])
    expect(names).not.toContain('archive')
  })

  it('uses one 100 MiB admission limit for every buffered format', () => {
    expect(detectViewerAdmissionKind('release.zip', 'application/octet-stream')).toBe('archive')
    expect(detectViewerAdmissionKind('portrait.mp4', 'video/mp4')).toBe('audio-video')
    expect(detectViewerAdmissionKind('unsafe.svg', 'image/svg+xml')).toBe('text')
    expect(detectViewerAdmissionKind('report.pdf', 'application/pdf')).toBe('pdf')
    expect(detectViewerAdmissionKind('data.sqlite', 'application/octet-stream')).toBe('sqlite')
    expect(detectViewerAdmissionKind('sheet.xlsx', 'application/octet-stream')).toBe('spreadsheet')
    expect(detectViewerAdmissionKind('records.tsv', 'text/tab-separated-values')).toBe('spreadsheet')

    expect(previewLimitForFile('release.zip', 'application/zip')).toBe(ARCHIVE_PREVIEW_MAX_SOURCE_BYTES)
    expect(previewLimitForFile('portrait.mp4', 'video/mp4')).toBeNull()
    expect(previewLimitForFile('unsafe.svg', 'image/svg+xml')).toBe(TEXT_PREVIEW_MAX_BYTES)
    expect(previewLimitForFile('photo.png', 'image/png')).toBe(IMAGE_PREVIEW_MAX_BYTES)
    expect(previewLimitForFile('report.pdf', 'application/pdf')).toBe(PDF_PREVIEW_MAX_BYTES)
    expect(previewLimitForFile('data.db', 'application/vnd.sqlite3')).toBe(SQLITE_PREVIEW_MAX_SOURCE_BYTES)
    expect(previewLimitForFile('sheet.xlsx', 'application/octet-stream')).toBe(SPREADSHEET_PREVIEW_MAX_SOURCE_BYTES)
    expect(previewLimitForFile('unknown.bin', 'application/octet-stream')).toBe(GENERAL_PREVIEW_MAX_BYTES)
    expect(new Set([
      ARCHIVE_PREVIEW_MAX_SOURCE_BYTES,
      SPREADSHEET_PREVIEW_MAX_SOURCE_BYTES,
      SQLITE_PREVIEW_MAX_SOURCE_BYTES,
      TEXT_PREVIEW_MAX_BYTES,
      IMAGE_PREVIEW_MAX_BYTES,
      PDF_PREVIEW_MAX_BYTES,
      GENERAL_PREVIEW_MAX_BYTES,
    ])).toEqual(new Set([BUFFERED_PREVIEW_MAX_BYTES]))
  })

  it('downloads a buffered file once with real progress and passes a File to upstream plugins', async () => {
    let streamController: ReadableStreamDefaultController<Uint8Array> | undefined
    const fetchMock = vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response(new ReadableStream<Uint8Array>({
      start(controller) { streamController = controller },
    }), { headers: { 'Content-Length': '4', 'Content-Type': 'image/svg+xml' } }))
    render(<ObjectFileViewer fileName="unsafe.svg" mimeType="image/svg+xml" size={1024} url="/signed/unsafe.svg" />)

    expect(screen.getByTestId('buffered-preview-download')).toBeInTheDocument()
    await act(async () => { streamController!.enqueue(new Uint8Array([1, 2])) })
    await waitFor(() => expect(screen.getByRole('progressbar', { name: '预览文件下载进度' })).toHaveAttribute('aria-valuenow', '50'))
    expect(screen.getByText('2 B / 4 B · 50%')).toBeInTheDocument()
    await act(async () => {
      streamController!.enqueue(new Uint8Array([3, 4]))
      streamController!.close()
    })

    expect(await screen.findByTestId('upstream-file-viewer')).toBeInTheDocument()
    expect(fetchMock).toHaveBeenCalledOnce()
    expect(fileViewerSpy).toHaveBeenCalledOnce()
    expect(fileViewerSpy.mock.calls[0][0]).toMatchObject({
      fileName: 'unsafe.svg',
      mimeType: 'text/xml',
      fit: 'contain',
      locale: 'zh-CN',
      theme: 'light',
      toolbar: false,
    })
    expect(fileViewerSpy.mock.calls[0][0].file).toBeInstanceOf(File)
    expect((fileViewerSpy.mock.calls[0][0].file as File).size).toBe(4)
    expect(normalizeViewerMimeType('vector.svg', 'application/octet-stream')).toBe('text/xml')
  })

  it('shows indeterminate byte progress without Content-Length', async () => {
    let streamController: ReadableStreamDefaultController<Uint8Array> | undefined
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response(new ReadableStream<Uint8Array>({
      start(controller) { streamController = controller },
    })))
    render(<ObjectFileViewer fileName="data.bin" mimeType="application/octet-stream" size={1024} url="/signed/data.bin" />)

    await act(async () => { streamController!.enqueue(new Uint8Array([1, 2, 3])) })
    await screen.findByText('3 B · 总大小未知')
    expect(screen.getByRole('progressbar', { name: '预览文件下载进度' })).not.toHaveAttribute('aria-valuenow')
    await act(async () => streamController!.close())
    expect(await screen.findByTestId('upstream-file-viewer')).toBeInTheDocument()
  })

  it('supports cancellation and a fresh retry after download failure', async () => {
    const fetchMock = vi.spyOn(globalThis, 'fetch')
      .mockImplementationOnce((_url, init) => new Promise((_resolve, reject) => {
        init?.signal?.addEventListener('abort', () => reject(new DOMException('cancelled', 'AbortError')))
      }))
      .mockResolvedValueOnce(new Response(new Uint8Array([1, 2]), { headers: { 'Content-Length': '2' } }))
    render(<ObjectFileViewer fileName="cancel.bin" mimeType="application/octet-stream" size={1024} url="/signed/cancel.bin" />)

    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    expect(await screen.findByText('预览下载已取消')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '重试' }))

    expect(await screen.findByTestId('upstream-file-viewer')).toBeInTheDocument()
    expect(fetchMock).toHaveBeenCalledTimes(2)
  })

  it('surfaces an HTTP failure and retries the same remote object', async () => {
    const fetchMock = vi.spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(new Response(null, { status: 503 }))
      .mockResolvedValueOnce(new Response(new Uint8Array([1]), { headers: { 'Content-Length': '1' } }))
    render(<ObjectFileViewer fileName="retry.pdf" mimeType="application/pdf" size={1024} url="/signed/retry.pdf" />)

    expect(await screen.findByText('预览文件下载失败')).toBeInTheDocument()
    expect(screen.getByText(/HTTP 503/)).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '重试' }))
    expect(await screen.findByTestId('upstream-file-viewer')).toBeInTheDocument()
    expect(fetchMock).toHaveBeenCalledTimes(2)
  })

  it('rejects oversized parser inputs before downloading or constructing the upstream viewer', () => {
    const fetchMock = vi.spyOn(globalThis, 'fetch')
    render(<ObjectFileViewer
      fileName="huge.xlsx"
      mimeType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
      size={BUFFERED_PREVIEW_MAX_BYTES + 1}
      url="/signed/huge.xlsx"
    />)

    expect(screen.getByText('文件超过在线预览上限')).toBeInTheDocument()
    expect(screen.getByText(/100 MB/)).toBeInTheDocument()
    expect(fetchMock).not.toHaveBeenCalled()
    expect(fileViewerSpy).not.toHaveBeenCalled()
  })

  it('allows streaming audio/video objects without the parser cap', () => {
    const fetchMock = vi.spyOn(globalThis, 'fetch')
    render(<ObjectFileViewer fileName="large.mp4" mimeType="video/mp4" size={2 * 1024 * 1024 * 1024} url="/signed/large.mp4" />)

    expect(screen.getByTestId('upstream-file-viewer')).toBeInTheDocument()
    expect(fileViewerSpy.mock.calls[0][0].file).toBe('/signed/large.mp4')
    expect(fetchMock).not.toHaveBeenCalled()
    expect(fileViewerSpy).toHaveBeenCalledOnce()
  })

  it('rejects oversized archives at the unified limit before the custom plugin can create its Worker', () => {
    render(<ObjectFileViewer
      fileName="backup.zip"
      mimeType="application/zip"
      size={ARCHIVE_PREVIEW_MAX_SOURCE_BYTES + 1}
      url="/signed/backup.zip"
    />)

    expect(screen.getByText(/100 MB/)).toBeInTheDocument()
    expect(fileViewerSpy).not.toHaveBeenCalled()
  })
})
