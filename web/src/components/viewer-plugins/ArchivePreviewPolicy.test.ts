import { describe, expect, it } from 'vitest'

import {
  ARCHIVE_PREVIEW_MAX_COMPRESSION_RATIO,
  ARCHIVE_PREVIEW_MAX_ENTRIES,
  ARCHIVE_PREVIEW_MAX_ENTRY_BYTES,
  ARCHIVE_PREVIEW_MAX_PATH_LENGTH,
  ARCHIVE_PREVIEW_MAX_SOURCE_BYTES,
  ARCHIVE_PREVIEW_MAX_TOTAL_BYTES,
  ARCHIVE_PREVIEW_RATIO_THRESHOLD_BYTES,
  ARCHIVE_EXPORT_MAX_ENTRIES,
  ARCHIVE_EXPORT_MAX_TOTAL_BYTES,
  ArchivePreviewPolicyError,
  assertArchiveSourceSize,
  createArchiveScanPolicy,
  evaluateArchiveEntry,
  normalizeArchiveEntryPath,
  selectArchiveExportEntries,
  type ArchiveScanPolicyState,
} from './ArchivePreviewPolicy'

const MEBIBYTE = 1024 * 1024

function accept(
  state: ArchiveScanPolicyState,
  entry: { path: string; size: number; directory?: boolean },
): ArchiveScanPolicyState {
  const decision = evaluateArchiveEntry(state, { directory: entry.directory ?? false, path: entry.path, size: entry.size })
  expect(decision.kind).toBe('accept')
  if (decision.kind !== 'accept') throw new Error('expected archive entry to be accepted')
  return decision.state
}

function expectPolicyError(action: () => unknown, code: ArchivePreviewPolicyError['code']) {
  try {
    action()
    throw new Error('expected ArchivePreviewPolicyError')
  } catch (cause) {
    expect(cause).toBeInstanceOf(ArchivePreviewPolicyError)
    expect((cause as ArchivePreviewPolicyError).code).toBe(code)
  }
}

describe('ArchivePreviewPolicy', () => {
  it('accepts a source at the 32 MiB limit and rejects invalid or larger sources', () => {
    expect(() => assertArchiveSourceSize(ARCHIVE_PREVIEW_MAX_SOURCE_BYTES)).not.toThrow()
    expectPolicyError(() => assertArchiveSourceSize(-1), 'invalid-source-size')
    expectPolicyError(() => assertArchiveSourceSize(ARCHIVE_PREVIEW_MAX_SOURCE_BYTES + 1), 'source-size-limit')
  })

  it('accepts at most 2000 entries and reports truncation before accepting another', () => {
    let state = createArchiveScanPolicy(8 * MEBIBYTE)
    for (let index = 0; index < ARCHIVE_PREVIEW_MAX_ENTRIES; index += 1) {
      state = accept(state, { path: `item-${index}.txt`, size: 1 })
    }

    const decision = evaluateArchiveEntry(state, { path: 'overflow.txt', size: 1, directory: false })
    expect(decision).toEqual({
      kind: 'truncate',
      reason: `压缩包条目超过 ${ARCHIVE_PREVIEW_MAX_ENTRIES} 条，仅显示前 ${ARCHIVE_PREVIEW_MAX_ENTRIES} 条。`,
    })
    expect(state.entryCount).toBe(ARCHIVE_PREVIEW_MAX_ENTRIES)
  })

  it('enforces path and single-entry declared-size limits', () => {
    const state = createArchiveScanPolicy(4 * MEBIBYTE)
    expect(() => accept(state, { path: 'a'.repeat(ARCHIVE_PREVIEW_MAX_PATH_LENGTH), size: ARCHIVE_PREVIEW_MAX_ENTRY_BYTES })).not.toThrow()
    expectPolicyError(
      () => evaluateArchiveEntry(state, { path: 'a'.repeat(ARCHIVE_PREVIEW_MAX_PATH_LENGTH + 1), size: 1, directory: false }),
      'path-length-limit',
    )
    expectPolicyError(
      () => evaluateArchiveEntry(state, { path: 'large.bin', size: ARCHIVE_PREVIEW_MAX_ENTRY_BYTES + 1, directory: false }),
      'entry-size-limit',
    )
    expectPolicyError(
      () => evaluateArchiveEntry(state, { path: 'unknown.bin', size: Number.NaN, directory: false }),
      'invalid-entry-size',
    )
  })

  it('ignores directory sizes and rejects totals above 512 MiB', () => {
    let state = createArchiveScanPolicy(ARCHIVE_PREVIEW_MAX_SOURCE_BYTES)
    state = accept(state, { path: 'folder/', size: Number.NaN, directory: true })
    expect(state.totalDeclaredSize).toBe(0)
    for (let index = 0; index < ARCHIVE_PREVIEW_MAX_TOTAL_BYTES / ARCHIVE_PREVIEW_MAX_ENTRY_BYTES; index += 1) {
      state = accept(state, { path: `part-${index}.bin`, size: ARCHIVE_PREVIEW_MAX_ENTRY_BYTES })
    }
    expect(state.totalDeclaredSize).toBe(ARCHIVE_PREVIEW_MAX_TOTAL_BYTES)
    expectPolicyError(
      () => evaluateArchiveEntry(state, { path: 'overflow.bin', size: 1, directory: false }),
      'total-size-limit',
    )
  })

  it('applies the compression-ratio limit only after 16 MiB of declared content', () => {
    const smallSource = 64 * 1024
    let state = createArchiveScanPolicy(smallSource)
    state = accept(state, { path: 'threshold.bin', size: ARCHIVE_PREVIEW_RATIO_THRESHOLD_BYTES })
    expect(state.totalDeclaredSize / smallSource).toBeGreaterThan(ARCHIVE_PREVIEW_MAX_COMPRESSION_RATIO)
    expectPolicyError(
      () => evaluateArchiveEntry(state, { path: 'one-more-byte.bin', size: 1, directory: false }),
      'compression-ratio-limit',
    )

    let boundary = createArchiveScanPolicy(MEBIBYTE)
    boundary = accept(boundary, { path: 'one.bin', size: 125 * MEBIBYTE })
    boundary = accept(boundary, { path: 'two.bin', size: 125 * MEBIBYTE })
    expect(boundary.totalDeclaredSize / boundary.sourceSize).toBe(ARCHIVE_PREVIEW_MAX_COMPRESSION_RATIO)
    expectPolicyError(
      () => evaluateArchiveEntry(boundary, { path: 'ratio-overflow.bin', size: 1, directory: false }),
      'compression-ratio-limit',
    )
  })

  it('normalizes safe relative paths and rejects traversal, absolute, and control-character paths', () => {
    expect(normalizeArchiveEntryPath('./docs\\guide/readme.md')).toBe('docs/guide/readme.md')
    expectPolicyError(() => normalizeArchiveEntryPath('../secret.txt'), 'unsafe-path')
    expectPolicyError(() => normalizeArchiveEntryPath('/etc/passwd'), 'unsafe-path')
    expectPolicyError(() => normalizeArchiveEntryPath('C:/Windows/system.ini'), 'unsafe-path')
    expectPolicyError(() => normalizeArchiveEntryPath('bad\0name.txt'), 'unsafe-path')
  })

  it('selects exact files and complete folder descendants for export', () => {
    const entries = [
      { path: 'docs', size: 0, directory: true },
      { path: 'docs/readme.txt', size: 10, directory: false },
      { path: 'docs/guides/start.md', size: 20, directory: false },
      { path: 'root.txt', size: 30, directory: false },
    ]
    expect(selectArchiveExportEntries(entries, 'docs/readme.txt', 'file', false).map((entry) => entry.path))
      .toEqual(['docs/readme.txt'])
    expect(selectArchiveExportEntries(entries, 'docs', 'folder', false).map((entry) => entry.path))
      .toEqual(['docs/readme.txt', 'docs/guides/start.md'])
    expectPolicyError(() => selectArchiveExportEntries(entries, 'docs', 'folder', true), 'export-truncated')
    expectPolicyError(() => selectArchiveExportEntries(entries, 'missing', 'file', false), 'export-not-found')
  })

  it('bounds folder export entry count and declared byte size', () => {
    const tooMany = Array.from({ length: ARCHIVE_EXPORT_MAX_ENTRIES + 1 }, (_, index) => ({
      path: `docs/${index}.txt`, size: 0, directory: false,
    }))
    expectPolicyError(() => selectArchiveExportEntries(tooMany, 'docs', 'folder', false), 'export-entry-limit')
    expectPolicyError(() => selectArchiveExportEntries([
      { path: 'docs/a.bin', size: ARCHIVE_EXPORT_MAX_TOTAL_BYTES, directory: false },
      { path: 'docs/b.bin', size: 1, directory: false },
    ], 'docs', 'folder', false), 'export-size-limit')
  })
})
