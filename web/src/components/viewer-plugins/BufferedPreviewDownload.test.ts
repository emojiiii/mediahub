import { afterEach, describe, expect, it, vi } from 'vitest'

import {
  BufferedPreviewDownloadError,
  downloadBufferedPreviewFile,
  type BufferedPreviewProgress,
} from './BufferedPreviewDownload'

afterEach(() => vi.restoreAllMocks())

describe('downloadBufferedPreviewFile', () => {
  it('streams one response into a File and reports determinate byte progress', async () => {
    const progress: BufferedPreviewProgress[] = []
    const fetchMock = vi.spyOn(globalThis, 'fetch').mockResolvedValue(responseFromChunks(
      [new Uint8Array([1, 2]), new Uint8Array([3, 4, 5])],
      { 'Content-Length': '5', 'Content-Type': 'application/x-test' },
    ))

    const file = await downloadBufferedPreviewFile({
      url: '/objects/report.bin',
      fileName: 'report.bin',
      mimeType: '',
      signal: new AbortController().signal,
      maxBytes: 10,
      onProgress: (value) => progress.push(value),
    })

    expect(fetchMock).toHaveBeenCalledOnce()
    expect(fetchMock).toHaveBeenCalledWith('/objects/report.bin', { signal: expect.any(AbortSignal) })
    expect(file).toBeInstanceOf(File)
    expect(file.name).toBe('report.bin')
    expect(file.type).toBe('application/x-test')
    expect(Array.from(new Uint8Array(await readFileBuffer(file)))).toEqual([1, 2, 3, 4, 5])
    expect(progress).toEqual([
      { loadedBytes: 0, totalBytes: 5 },
      { loadedBytes: 2, totalBytes: 5 },
      { loadedBytes: 5, totalBytes: 5 },
    ])
  })

  it('keeps progress indeterminate when Content-Length is unavailable', async () => {
    const progress: BufferedPreviewProgress[] = []
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(responseFromChunks([new Uint8Array([1, 2, 3])]))

    await downloadBufferedPreviewFile({
      url: '/objects/unknown.bin',
      fileName: 'unknown.bin',
      mimeType: 'application/octet-stream',
      signal: new AbortController().signal,
      maxBytes: 10,
      onProgress: (value) => progress.push(value),
    })

    expect(progress).toEqual([
      { loadedBytes: 0, totalBytes: null },
      { loadedBytes: 3, totalBytes: null },
    ])
  })

  it('rejects an oversized Content-Length before reading the body', async () => {
    const cancelled = vi.fn()
    const body = new ReadableStream<Uint8Array>({ cancel: cancelled })
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response(body, { headers: { 'Content-Length': '11' } }))

    await expect(downloadBufferedPreviewFile({
      url: '/objects/large.bin',
      fileName: 'large.bin',
      mimeType: '',
      signal: new AbortController().signal,
      maxBytes: 10,
    })).rejects.toMatchObject({ code: 'size-limit' } satisfies Partial<BufferedPreviewDownloadError>)
    expect(cancelled).toHaveBeenCalledOnce()
  })

  it('cancels a stream as soon as cumulative bytes exceed the limit', async () => {
    const cancelled = vi.fn()
    let index = 0
    const chunks = [new Uint8Array([1, 2, 3]), new Uint8Array([4, 5, 6])]
    const body = new ReadableStream<Uint8Array>({
      pull(controller) {
        const chunk = chunks[index++]
        if (chunk) controller.enqueue(chunk)
      },
      cancel: cancelled,
    })
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response(body))

    await expect(downloadBufferedPreviewFile({
      url: '/objects/growing.bin',
      fileName: 'growing.bin',
      mimeType: '',
      signal: new AbortController().signal,
      maxBytes: 5,
    })).rejects.toMatchObject({ code: 'size-limit' })
    expect(cancelled).toHaveBeenCalledWith('preview-size-limit')
  })
})

function responseFromChunks(chunks: Uint8Array[], headers?: HeadersInit): Response {
  let index = 0
  return new Response(new ReadableStream<Uint8Array>({
    pull(controller) {
      const chunk = chunks[index++]
      if (chunk) controller.enqueue(chunk)
      else controller.close()
    },
  }), { headers })
}

function readFileBuffer(file: File): Promise<ArrayBuffer> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onerror = () => reject(reader.error)
    reader.onload = () => resolve(reader.result as ArrayBuffer)
    reader.readAsArrayBuffer(file)
  })
}
