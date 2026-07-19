export const UPLOAD_HASH_CHUNK_BYTES = 8 * 1024 * 1024

export async function sha256File(
  file: File,
  signal?: AbortSignal,
  chunkBytes = UPLOAD_HASH_CHUNK_BYTES,
): Promise<string> {
  if (!Number.isSafeInteger(chunkBytes) || chunkBytes <= 0) throw new Error('SHA-256 chunk size must be positive')
  throwIfAborted(signal)
  const { createSHA256 } = await import('hash-wasm')
  const hasher = await createSHA256()
  hasher.init()
  for (let offset = 0; offset < file.size; offset += chunkBytes) {
    throwIfAborted(signal)
    const chunk = await readBlob(file.slice(offset, Math.min(file.size, offset + chunkBytes)), signal)
    hasher.update(new Uint8Array(chunk))
  }
  throwIfAborted(signal)
  return hasher.digest('hex')
}

function readBlob(blob: Blob, signal?: AbortSignal): Promise<ArrayBuffer> {
  if (typeof blob.arrayBuffer === 'function') return blob.arrayBuffer()
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    const abort = () => reader.abort()
    signal?.addEventListener('abort', abort, { once: true })
    reader.onload = () => {
      signal?.removeEventListener('abort', abort)
      resolve(reader.result as ArrayBuffer)
    }
    reader.onerror = () => {
      signal?.removeEventListener('abort', abort)
      reject(reader.error ?? new Error('文件分片读取失败'))
    }
    reader.onabort = () => {
      signal?.removeEventListener('abort', abort)
      reject(signal?.reason instanceof Error ? signal.reason : new DOMException('上传已取消', 'AbortError'))
    }
    reader.readAsArrayBuffer(blob)
  })
}

function throwIfAborted(signal?: AbortSignal) {
  if (!signal?.aborted) return
  if (signal.reason instanceof Error) throw signal.reason
  throw new DOMException('上传已取消', 'AbortError')
}
