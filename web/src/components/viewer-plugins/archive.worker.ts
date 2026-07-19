import { ArchiveReader } from 'libarchive.js/src/webworker/archive-reader.js'
import { getWasmModule } from 'libarchive.js/src/webworker/wasm-module.js'

import { createStoredArchiveFolderZip } from './ArchiveFolderZip'
import { configureArchiveWasmLocation } from './ArchiveWasmLocator'
import {
  ARCHIVE_PREVIEW_MAX_SOURCE_BYTES,
  ArchivePreviewPolicyError,
  assertArchiveSourceSize,
  createArchiveScanPolicy,
  evaluateArchiveEntry,
  normalizeArchiveEntryPath,
  selectArchiveExportEntries,
} from './ArchivePreviewPolicy'
import {
  isArchiveWorkerRequest,
  type ArchiveEntrySummary,
  type ArchiveExportRequest,
  type ArchiveScanRequest,
  type ArchiveScanResult,
  type ArchiveWorkerResponse,
} from './archive-protocol'

type LibArchiveEntry = {
  path?: unknown
  size?: unknown
  type?: unknown
}

type ArchiveRuntime = {
  HEAPU8: Uint8Array
  runCode: {
    openArchive(data: number, size: number, password: string | null, locale: string): number
    getNextEntry(archive: number): number
    getEntrySize(entry: number): number
    getEntryName(entry: number): string
    getEntryType(entry: number): number
    entryIsEncrypted(entry: number): number
    hasEncryptedEntries(archive: number): number
    getFileData(archive: number, size: number): number
    getError(archive: number): string | null
    skipEntry(archive: number): number
    closeArchive(archive: number): void
    malloc(size: number): number
    free(pointer: number): void
  }
}

type ExtractedArchiveFile = {
  path: string
  data: Uint8Array
}

class ArchivePasswordChallenge extends Error {
  constructor(readonly result?: ArchiveScanResult) {
    super('Incorrect passphrase')
    this.name = 'ArchivePasswordChallenge'
  }
}

type WorkerScope = {
  onmessage: ((event: MessageEvent<unknown>) => void) | null
  postMessage(message: ArchiveWorkerResponse, transfer?: Transferable[]): void
}

const FILE_TYPE = 32768
const DIRECTORY_TYPE = 16384
const ARCHIVE_LOCALE = 'en_US.UTF-8'

const workerScope = self as unknown as WorkerScope
let requestQueue = Promise.resolve()
let wasmModulePromise: Promise<unknown> | undefined

configureArchiveWasmLocation()

workerScope.onmessage = (event) => {
  requestQueue = requestQueue.then(() => handleMessage(event.data))
}

async function handleMessage(message: unknown): Promise<void> {
  if (!isArchiveWorkerRequest(message)) {
    postFailure('scan', '压缩包请求无效。')
    return
  }

  try {
    if (message.type === 'scan') {
      const result = await scanArchive(message)
      if (result.encrypted === true && !message.password) {
        postPasswordRequired(false, result)
      } else {
        workerScope.postMessage({ type: 'scan-success', result })
      }
      return
    }

    const exported = await exportArchiveEntry(message)
    const data = toTransferableBuffer(exported.data)
    workerScope.postMessage({
      type: 'export-success',
      path: message.path,
      fileName: exported.fileName,
      mimeType: exported.mimeType,
      data,
    }, [data])
  } catch (cause) {
    if (cause instanceof ArchivePasswordChallenge) {
      postPasswordRequired(true, cause.result)
      return
    }
    if (isPasswordFailure(cause)) {
      postPasswordRequired(Boolean(message.password))
      return
    }
    postFailure(message.type, archiveErrorMessage(cause), message.type === 'export' ? message.path : undefined)
  }
}

async function scanArchive(request: ArchiveScanRequest): Promise<ArchiveScanResult> {
  assertArchiveSourceSize(request.sourceSize)
  const file = await fetchArchiveFile(request.url)
  return inspectArchiveForRequest(file, request.sourceSize, request.password)
}

async function inspectArchiveForRequest(
  file: File,
  sourceSize: number,
  password?: string,
): Promise<ArchiveScanResult> {
  let result: ArchiveScanResult
  try {
    result = await inspectArchive(file, sourceSize, password)
  } catch (cause) {
    if (!password || !shouldTrySevenZip(cause)) throw cause
    try {
      return await inspectEncryptedArchiveWithSevenZip(file, sourceSize, password)
    } catch (fallbackCause) {
      if (isPasswordFailure(fallbackCause)) throw new ArchivePasswordChallenge()
      throw fallbackCause
    }
  }

  if (password && (result.encrypted === true || result.entries.length === 0)) {
    try {
      return await inspectEncryptedArchiveWithSevenZip(file, sourceSize, password)
    } catch (cause) {
      if (isPasswordFailure(cause)) throw new ArchivePasswordChallenge(result)
      throw cause
    }
  }
  return result
}

async function inspectEncryptedArchiveWithSevenZip(
  file: File,
  sourceSize: number,
  password: string,
): Promise<ArchiveScanResult> {
  const { inspectWithSevenZip } = await import('./SevenZipArchive')
  const result = await inspectWithSevenZip(file, password)
  let policy = createArchiveScanPolicy(sourceSize)
  const entries: ArchiveEntrySummary[] = []
  const seenPaths = new Set<string>()
  let truncationReason: string | undefined

  for (const entry of result.entries) {
    if (seenPaths.has(entry.path)) {
      throw new ArchivePreviewPolicyError('unsafe-path', '压缩包中存在重复路径，无法安全导出。')
    }
    seenPaths.add(entry.path)
    const decision = evaluateArchiveEntry(policy, entry)
    if (decision.kind === 'truncate') {
      truncationReason = decision.reason
      break
    }
    entries.push(decision.entry)
    policy = decision.state
  }

  return {
    entries,
    entryCount: policy.entryCount,
    totalDeclaredSize: policy.totalDeclaredSize,
    truncated: truncationReason !== undefined,
    encrypted: true,
    ...(truncationReason ? { truncationReason } : {}),
  }
}

async function inspectArchive(file: File, sourceSize: number, password?: string): Promise<ArchiveScanResult> {
  const encrypted = await detectArchiveEncryption(file, password)
  const reader = new ArchiveReader(await getArchiveWasmModule())
  let scanFailure: unknown

  try {
    if (password) reader.setPassphrase(password)
    await reader.open(file)
    let policy = createArchiveScanPolicy(sourceSize)
    const entries: ArchiveEntrySummary[] = []
    const seenPaths = new Set<string>()
    let truncationReason: string | undefined

    for (const rawEntry of reader.entries(true) as Generator<LibArchiveEntry>) {
      const entry = normalizeEntry(rawEntry)
      if (!entry) continue
      if (!entry.path) continue
      if (seenPaths.has(entry.path)) {
        throw new ArchivePreviewPolicyError('unsafe-path', '压缩包中存在重复路径，无法安全导出。')
      }
      seenPaths.add(entry.path)
      const decision = evaluateArchiveEntry(policy, entry)
      if (decision.kind === 'truncate') {
        truncationReason = decision.reason
        break
      }
      entries.push(decision.entry)
      policy = decision.state
    }

    return {
      entries,
      entryCount: policy.entryCount,
      totalDeclaredSize: policy.totalDeclaredSize,
      truncated: truncationReason !== undefined,
      encrypted,
      ...(truncationReason ? { truncationReason } : {}),
    }
  } catch (cause) {
    scanFailure = cause
    throw cause
  } finally {
    try {
      reader.close()
    } catch (closeError) {
      if (scanFailure === undefined) throw closeError
    }
  }
}

async function detectArchiveEncryption(file: File, password?: string): Promise<boolean> {
  const module = await getArchiveWasmModule() as ArchiveRuntime
  const source = new Uint8Array(await file.arrayBuffer())
  const sourcePointer = module.runCode.malloc(source.byteLength)
  module.HEAPU8.set(source, sourcePointer)
  let archive = 0

  try {
    archive = module.runCode.openArchive(sourcePointer, source.byteLength, password || null, ARCHIVE_LOCALE)
    if (!archive) throw new Error(password ? 'Incorrect passphrase' : 'Unable to open archive')

    let encrypted = module.runCode.hasEncryptedEntries(archive) > 0
    while (true) {
      const entryPointer = module.runCode.getNextEntry(archive)
      if (!entryPointer) break
      encrypted = encrypted
        || module.runCode.entryIsEncrypted(entryPointer) > 0
        || module.runCode.hasEncryptedEntries(archive) > 0
      module.runCode.skipEntry(archive)
    }

    encrypted = encrypted || module.runCode.hasEncryptedEntries(archive) > 0
    const readError = module.runCode.getError(archive)
    if (readError && !encrypted) throw new Error(readError)
    return encrypted
  } finally {
    if (archive) module.runCode.closeArchive(archive)
    module.runCode.free(sourcePointer)
  }
}

async function exportArchiveEntry(request: ArchiveExportRequest): Promise<{
  fileName: string
  mimeType: string
  data: Uint8Array
}> {
  assertArchiveSourceSize(request.sourceSize)
  const file = await fetchArchiveFile(request.url)
  const result = await inspectArchiveForRequest(file, request.sourceSize, request.password)
  if (result.encrypted === true && !request.password) throw new Error('Passphrase required for encrypted archive')

  const normalizedTarget = normalizeArchiveEntryPath(request.path)
  const selected = selectArchiveExportEntries(result.entries, normalizedTarget, request.target, result.truncated)
  const extracted = result.encrypted && request.password
    ? await extractEncryptedFiles(file, request.password, selected)
    : await extractSelectedFiles(file, request.password, selected)

  if (request.target === 'file') {
    return {
      fileName: safeDownloadName(selected[0].path),
      mimeType: 'application/octet-stream',
      data: extracted[0].data,
    }
  }

  const folderName = safeDownloadName(normalizedTarget)
  const data = await createStoredArchiveFolderZip(extracted, normalizedTarget)
  return { fileName: `${folderName}.zip`, mimeType: 'application/zip', data }
}

async function extractEncryptedFiles(
  file: File,
  password: string,
  selected: ArchiveEntrySummary[],
): Promise<ExtractedArchiveFile[]> {
  const { extractWithSevenZip } = await import('./SevenZipArchive')
  return extractWithSevenZip(file, password, selected)
}

async function extractSelectedFiles(
  file: File,
  password: string | undefined,
  selected: ArchiveEntrySummary[],
): Promise<ExtractedArchiveFile[]> {
  const module = await getArchiveWasmModule() as ArchiveRuntime
  const source = new Uint8Array(await file.arrayBuffer())
  const sourcePointer = module.runCode.malloc(source.byteLength)
  module.HEAPU8.set(source, sourcePointer)
  let archive = 0

  try {
    archive = module.runCode.openArchive(sourcePointer, source.byteLength, password || null, ARCHIVE_LOCALE)
    if (!archive) throw new Error(password ? 'Incorrect passphrase or unsupported archive' : 'Unable to open archive')

    const selectedByPath = new Map(selected.map((entry) => [entry.path, entry]))
    const extracted: ExtractedArchiveFile[] = []
    while (selectedByPath.size > 0) {
      const entryPointer = module.runCode.getNextEntry(archive)
      if (!entryPointer) break
      const entryType = module.runCode.getEntryType(entryPointer)
      if (entryType !== FILE_TYPE) {
        module.runCode.skipEntry(archive)
        continue
      }

      const path = normalizeArchiveEntryPath(module.runCode.getEntryName(entryPointer))
      const selectedEntry = selectedByPath.get(path)
      if (!selectedEntry) {
        module.runCode.skipEntry(archive)
        continue
      }

      const size = module.runCode.getEntrySize(entryPointer)
      if (size !== selectedEntry.size) {
        throw new ArchivePreviewPolicyError('invalid-entry-size', '压缩包条目大小在导出时发生变化，已停止导出。')
      }
      const dataPointer = module.runCode.getFileData(archive, size)
      if (dataPointer < 0) throw new Error(module.runCode.getError(archive) || 'Archive extraction failed')
      try {
        extracted.push({ path, data: module.HEAPU8.slice(dataPointer, dataPointer + size) })
      } finally {
        module.runCode.free(dataPointer)
      }
      selectedByPath.delete(path)
    }

    if (selectedByPath.size > 0) {
      throw new ArchivePreviewPolicyError('export-not-found', '压缩包中的目标文件未能完整导出。')
    }
    return extracted
  } finally {
    if (archive) module.runCode.closeArchive(archive)
    module.runCode.free(sourcePointer)
  }
}

function normalizeEntry(entry: LibArchiveEntry): ArchiveEntrySummary | null {
  if (entry.type !== 'FILE' && entry.type !== 'DIR') return null
  const path = normalizeArchiveEntryPath(typeof entry.path === 'string' ? entry.path : '')
  return {
    path,
    size: typeof entry.size === 'number' ? entry.size : Number.NaN,
    directory: entry.type === 'DIR',
  }
}

async function fetchArchiveFile(url: string): Promise<File> {
  const response = await fetch(url)
  if (!response.ok) throw new Error(`压缩文件读取失败（HTTP ${response.status}）`)

  const contentLength = response.headers.get('Content-Length')
  if (contentLength) {
    const declaredLength = Number(contentLength)
    if (Number.isFinite(declaredLength)) assertArchiveSourceSize(declaredLength)
  }

  if (!response.body) {
    const buffer = await response.arrayBuffer()
    assertArchiveSourceSize(buffer.byteLength)
    return new File([buffer], 'archive', { type: 'application/octet-stream' })
  }

  const reader = response.body.getReader()
  const chunks: Uint8Array[] = []
  let totalBytes = 0
  while (true) {
    const { done, value } = await reader.read()
    if (done) break
    totalBytes += value.byteLength
    if (totalBytes > ARCHIVE_PREVIEW_MAX_SOURCE_BYTES) {
      await reader.cancel()
      throw new ArchivePreviewPolicyError('source-size-limit', '压缩文件超过 100 MB 的在线预览上限。')
    }
    chunks.push(value)
  }

  const bytes = new Uint8Array(totalBytes)
  let offset = 0
  for (const chunk of chunks) {
    bytes.set(chunk, offset)
    offset += chunk.byteLength
  }
  return new File([bytes], 'archive', { type: 'application/octet-stream' })
}

function getArchiveWasmModule(): Promise<unknown> {
  if (!wasmModulePromise) {
    wasmModulePromise = new Promise((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error('压缩包解析引擎加载超时。')), 30_000)
      try {
        getWasmModule((module) => {
          clearTimeout(timeout)
          resolve(module)
        })
      } catch (cause) {
        clearTimeout(timeout)
        reject(cause)
      }
    })
  }
  return wasmModulePromise
}

function isPasswordFailure(cause: unknown): boolean {
  if (cause instanceof Error && cause.name === 'SevenZipPasswordError') return true
  const message = cause instanceof Error ? cause.message : String(cause || '')
  return !/unsupported/i.test(message) && /passphrase|password|decrypt|encrypted/i.test(message)
}

function shouldTrySevenZip(cause: unknown): boolean {
  const message = cause instanceof Error ? cause.message : String(cause || '')
  return /passphrase|password|decrypt|encrypted/i.test(message)
}

function archiveErrorMessage(cause: unknown): string {
  if (cause instanceof ArchivePreviewPolicyError) return cause.message
  const message = cause instanceof Error ? cause.message : String(cause || '')
  return message ? `压缩包处理失败：${message}` : '压缩包处理失败。'
}

function postPasswordRequired(invalid: boolean, result?: ArchiveScanResult): void {
  workerScope.postMessage({
    type: 'password-required',
    invalid,
    error: invalid ? '密码不正确，请重新输入。' : '此压缩包已加密，需要密码。',
    ...(result ? { result } : {}),
  })
}

function postFailure(operation: 'scan' | 'export', error: string, path?: string): void {
  workerScope.postMessage({ type: 'error', operation, error, ...(path ? { path } : {}) })
}

function safeDownloadName(path: string): string {
  const segments = path.split('/').filter(Boolean)
  const segment = segments[segments.length - 1] || 'archive'
  const sanitized = segment.replace(/[<>:"/\\|?*\x00-\x1f\x7f]/g, '_').replace(/[. ]+$/g, '').trim()
  return (sanitized || 'archive').slice(0, 180)
}

function toTransferableBuffer(bytes: Uint8Array): ArrayBuffer {
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer
}
