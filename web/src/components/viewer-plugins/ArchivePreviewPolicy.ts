import type { ArchiveEntrySummary } from './archive-protocol'
import { BUFFERED_PREVIEW_MAX_BYTES } from './BufferedPreviewPolicy'

const MEBIBYTE = 1024 * 1024

export const ARCHIVE_PREVIEW_MAX_SOURCE_BYTES = BUFFERED_PREVIEW_MAX_BYTES
export const ARCHIVE_PREVIEW_MAX_ENTRIES = 2_000
export const ARCHIVE_PREVIEW_MAX_ENTRY_BYTES = 128 * MEBIBYTE
export const ARCHIVE_PREVIEW_MAX_TOTAL_BYTES = 512 * MEBIBYTE
export const ARCHIVE_PREVIEW_MAX_PATH_LENGTH = 1_024
export const ARCHIVE_PREVIEW_RATIO_THRESHOLD_BYTES = 16 * MEBIBYTE
export const ARCHIVE_PREVIEW_MAX_COMPRESSION_RATIO = 250
export const ARCHIVE_EXPORT_MAX_ENTRIES = 1_000
export const ARCHIVE_EXPORT_MAX_TOTAL_BYTES = 128 * MEBIBYTE

export type ArchivePreviewPolicyErrorCode =
  | 'invalid-source-size'
  | 'source-size-limit'
  | 'invalid-entry-size'
  | 'entry-size-limit'
  | 'total-size-limit'
  | 'path-length-limit'
  | 'compression-ratio-limit'
  | 'unsafe-path'
  | 'export-not-found'
  | 'export-truncated'
  | 'export-entry-limit'
  | 'export-size-limit'

export class ArchivePreviewPolicyError extends Error {
  readonly code: ArchivePreviewPolicyErrorCode

  constructor(code: ArchivePreviewPolicyErrorCode, message: string) {
    super(message)
    this.name = 'ArchivePreviewPolicyError'
    this.code = code
  }
}

export type ArchiveScanPolicyState = Readonly<{
  sourceSize: number
  entryCount: number
  totalDeclaredSize: number
}>

export type ArchiveEntryPolicyDecision =
  | { kind: 'accept'; state: ArchiveScanPolicyState; entry: ArchiveEntrySummary }
  | { kind: 'truncate'; reason: string }

export function assertArchiveSourceSize(sourceSize: number): void {
  if (!Number.isSafeInteger(sourceSize) || sourceSize < 0) {
    throw new ArchivePreviewPolicyError('invalid-source-size', '压缩文件大小无效，无法安全预览。')
  }
  if (sourceSize > ARCHIVE_PREVIEW_MAX_SOURCE_BYTES) {
    throw new ArchivePreviewPolicyError('source-size-limit', '压缩文件超过 100 MB 的在线预览上限。')
  }
}

export function createArchiveScanPolicy(sourceSize: number): ArchiveScanPolicyState {
  assertArchiveSourceSize(sourceSize)
  return { sourceSize, entryCount: 0, totalDeclaredSize: 0 }
}

export function evaluateArchiveEntry(
  state: ArchiveScanPolicyState,
  entry: ArchiveEntrySummary,
): ArchiveEntryPolicyDecision {
  if (state.entryCount >= ARCHIVE_PREVIEW_MAX_ENTRIES) {
    return {
      kind: 'truncate',
      reason: `压缩包条目超过 ${ARCHIVE_PREVIEW_MAX_ENTRIES} 条，仅显示前 ${ARCHIVE_PREVIEW_MAX_ENTRIES} 条。`,
    }
  }

  if (entry.path.length > ARCHIVE_PREVIEW_MAX_PATH_LENGTH) {
    throw new ArchivePreviewPolicyError(
      'path-length-limit',
      `压缩包中存在超过 ${ARCHIVE_PREVIEW_MAX_PATH_LENGTH} 个字符的路径，已停止预览。`,
    )
  }

  const declaredSize = entry.directory ? 0 : entry.size
  if (!Number.isSafeInteger(declaredSize) || declaredSize < 0) {
    throw new ArchivePreviewPolicyError('invalid-entry-size', '压缩包中存在大小无效的条目，已停止预览。')
  }
  if (declaredSize > ARCHIVE_PREVIEW_MAX_ENTRY_BYTES) {
    throw new ArchivePreviewPolicyError('entry-size-limit', '压缩包中存在声明大小超过 128 MB 的条目，已停止预览。')
  }

  const totalDeclaredSize = state.totalDeclaredSize + declaredSize
  if (!Number.isSafeInteger(totalDeclaredSize) || totalDeclaredSize > ARCHIVE_PREVIEW_MAX_TOTAL_BYTES) {
    throw new ArchivePreviewPolicyError('total-size-limit', '压缩包条目声明总大小超过 512 MB，已停止预览。')
  }
  if (
    totalDeclaredSize > ARCHIVE_PREVIEW_RATIO_THRESHOLD_BYTES
    && compressionRatio(totalDeclaredSize, state.sourceSize) > ARCHIVE_PREVIEW_MAX_COMPRESSION_RATIO
  ) {
    throw new ArchivePreviewPolicyError('compression-ratio-limit', '压缩包的声明压缩比超过 250，已停止预览。')
  }

  return {
    kind: 'accept',
    entry: { ...entry, size: declaredSize },
    state: {
      sourceSize: state.sourceSize,
      entryCount: state.entryCount + 1,
      totalDeclaredSize,
    },
  }
}

export function normalizeArchiveEntryPath(path: string): string {
  if (path.length > ARCHIVE_PREVIEW_MAX_PATH_LENGTH) {
    throw new ArchivePreviewPolicyError(
      'path-length-limit',
      `压缩包中存在超过 ${ARCHIVE_PREVIEW_MAX_PATH_LENGTH} 个字符的路径，已停止预览。`,
    )
  }
  if (/[\x00-\x1f\x7f]/.test(path)) {
    throw new ArchivePreviewPolicyError('unsafe-path', '压缩包中存在包含控制字符的不安全路径，已停止预览。')
  }

  const separatorsNormalized = path.replace(/\\/g, '/')
  if (separatorsNormalized.startsWith('/') || /^[A-Za-z]:($|\/)/.test(separatorsNormalized)) {
    throw new ArchivePreviewPolicyError('unsafe-path', '压缩包中存在绝对路径，已停止预览。')
  }

  const segments = separatorsNormalized.split('/').filter((segment) => segment.length > 0 && segment !== '.')
  if (segments.some((segment) => segment === '..')) {
    throw new ArchivePreviewPolicyError('unsafe-path', '压缩包中存在越过目录边界的路径，已停止预览。')
  }
  const normalized = segments.join('/')
  if (normalized.length > ARCHIVE_PREVIEW_MAX_PATH_LENGTH) {
    throw new ArchivePreviewPolicyError(
      'path-length-limit',
      `压缩包中存在超过 ${ARCHIVE_PREVIEW_MAX_PATH_LENGTH} 个字符的路径，已停止预览。`,
    )
  }
  return normalized
}

export function selectArchiveExportEntries(
  entries: ArchiveEntrySummary[],
  targetPath: string,
  target: 'file' | 'folder',
  truncated: boolean,
): ArchiveEntrySummary[] {
  const normalizedTarget = normalizeArchiveEntryPath(targetPath)
  if (!normalizedTarget) {
    throw new ArchivePreviewPolicyError('export-not-found', '没有找到要导出的压缩包条目。')
  }

  let selected: ArchiveEntrySummary[]
  if (target === 'file') {
    const entry = entries.find((candidate) => !candidate.directory && candidate.path === normalizedTarget)
    if (!entry) throw new ArchivePreviewPolicyError('export-not-found', '没有找到要导出的文件。')
    selected = [entry]
  } else {
    if (truncated) {
      throw new ArchivePreviewPolicyError('export-truncated', '目录列表不完整，不能导出可能缺少文件的目录。')
    }
    const prefix = `${normalizedTarget}/`
    selected = entries.filter((candidate) => !candidate.directory && candidate.path.startsWith(prefix))
    if (selected.length === 0) {
      throw new ArchivePreviewPolicyError('export-not-found', '这个目录中没有可导出的文件。')
    }
  }

  if (selected.length > ARCHIVE_EXPORT_MAX_ENTRIES) {
    throw new ArchivePreviewPolicyError(
      'export-entry-limit',
      `一次最多导出 ${ARCHIVE_EXPORT_MAX_ENTRIES} 个文件。`,
    )
  }
  const totalBytes = selected.reduce((total, entry) => total + entry.size, 0)
  if (!Number.isSafeInteger(totalBytes) || totalBytes > ARCHIVE_EXPORT_MAX_TOTAL_BYTES) {
    throw new ArchivePreviewPolicyError('export-size-limit', '一次导出的文件总大小不能超过 128 MB。')
  }
  return selected
}

function compressionRatio(totalDeclaredSize: number, sourceSize: number): number {
  return sourceSize === 0 ? Number.POSITIVE_INFINITY : totalDeclaredSize / sourceSize
}
