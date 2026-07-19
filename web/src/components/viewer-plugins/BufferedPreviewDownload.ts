import { BUFFERED_PREVIEW_MAX_BYTES } from './BufferedPreviewPolicy'

export type BufferedPreviewProgress = {
  loadedBytes: number
  totalBytes: number | null
}

export class BufferedPreviewDownloadError extends Error {
  constructor(
    message: string,
    readonly code: 'http' | 'size-limit',
  ) {
    super(message)
    this.name = 'BufferedPreviewDownloadError'
  }
}

export type BufferedPreviewDownloadOptions = {
  url: string
  fileName: string
  mimeType: string
  signal: AbortSignal
  maxBytes?: number
  onProgress?: (progress: BufferedPreviewProgress) => void
}

export async function downloadBufferedPreviewFile({
  url,
  fileName,
  mimeType,
  signal,
  maxBytes = BUFFERED_PREVIEW_MAX_BYTES,
  onProgress,
}: BufferedPreviewDownloadOptions): Promise<File> {
  const response = await fetch(url, { signal })
  if (!response.ok) {
    throw new BufferedPreviewDownloadError(`预览文件下载失败（HTTP ${response.status}）`, 'http')
  }

  const totalBytes = parseContentLength(response.headers.get('Content-Length'))
  if (totalBytes !== null && totalBytes > maxBytes) {
    await cancelBody(response.body)
    throw sizeLimitError(maxBytes)
  }
  onProgress?.({ loadedBytes: 0, totalBytes })

  const responseMimeType = response.headers.get('Content-Type')?.split(';', 1)[0].trim() || ''
  const fileType = mimeType || responseMimeType || 'application/octet-stream'
  if (!response.body) {
    const buffer = await response.arrayBuffer()
    if (buffer.byteLength > maxBytes) throw sizeLimitError(maxBytes)
    onProgress?.({ loadedBytes: buffer.byteLength, totalBytes: totalBytes ?? buffer.byteLength })
    return new File([buffer], fileName, { type: fileType })
  }

  const reader = response.body.getReader()
  const chunks: Uint8Array<ArrayBuffer>[] = []
  let loadedBytes = 0
  try {
    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      loadedBytes += value.byteLength
      if (loadedBytes > maxBytes) {
        try {
          await reader.cancel('preview-size-limit')
        } catch {
          // Preserve the deterministic size-limit error even if stream cancellation fails.
        }
        throw sizeLimitError(maxBytes)
      }
      chunks.push(new Uint8Array(value))
      onProgress?.({ loadedBytes, totalBytes })
    }
  } finally {
    reader.releaseLock()
  }

  return new File(chunks, fileName, { type: fileType })
}

function parseContentLength(value: string | null): number | null {
  if (!value) return null
  const parsed = Number(value)
  return Number.isSafeInteger(parsed) && parsed >= 0 ? parsed : null
}

async function cancelBody(body: ReadableStream<Uint8Array> | null): Promise<void> {
  try {
    await body?.cancel('preview-size-limit')
  } catch {
    // The response is still rejected by the caller's admission check.
  }
}

function sizeLimitError(maxBytes: number): BufferedPreviewDownloadError {
  return new BufferedPreviewDownloadError(`文件超过 ${formatDownloadBytes(maxBytes)} 的在线预览上限`, 'size-limit')
}

export function formatDownloadBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '--'
  if (bytes < 1024) return `${Math.round(bytes)} B`
  const units = ['KB', 'MB', 'GB']
  let value = bytes / 1024
  let unitIndex = 0
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024
    unitIndex += 1
  }
  const precision = value >= 10 ? 0 : 1
  return `${value.toFixed(precision).replace(/\.0$/, '')} ${units[unitIndex]}`
}

export function isDownloadAbortError(cause: unknown): boolean {
  return cause instanceof DOMException && cause.name === 'AbortError'
}
