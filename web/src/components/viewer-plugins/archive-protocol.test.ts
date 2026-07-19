import { describe, expect, it } from 'vitest'

import { ARCHIVE_MAX_PASSWORD_LENGTH, isArchiveWorkerRequest } from './archive-protocol'

describe('archive protocol', () => {
  it('accepts bounded scans and exports', () => {
    expect(isArchiveWorkerRequest({ type: 'scan', url: 'blob:archive', sourceSize: 10 })).toBe(true)
    expect(isArchiveWorkerRequest({ type: 'scan', url: 'blob:archive', sourceSize: 10, password: '秘密' })).toBe(true)
    expect(isArchiveWorkerRequest({ type: 'export', url: 'blob:archive', sourceSize: 10, path: 'docs/a.txt', target: 'file' })).toBe(true)
    expect(isArchiveWorkerRequest({ type: 'export', url: 'blob:archive', sourceSize: 10, path: 'docs', target: 'folder' })).toBe(true)
  })

  it('rejects missing targets, invalid operations, and unsafe password payloads', () => {
    expect(isArchiveWorkerRequest({ type: 'export', url: 'blob:archive', sourceSize: 10, path: '', target: 'file' })).toBe(false)
    expect(isArchiveWorkerRequest({ type: 'export', url: 'blob:archive', sourceSize: 10, path: 'a', target: 'archive' })).toBe(false)
    expect(isArchiveWorkerRequest({ type: 'scan', url: '', sourceSize: 10 })).toBe(false)
    expect(isArchiveWorkerRequest({ type: 'scan', url: 'blob:archive', sourceSize: 10, password: 'a'.repeat(ARCHIVE_MAX_PASSWORD_LENGTH + 1) })).toBe(false)
    expect(isArchiveWorkerRequest({ type: 'scan', url: 'blob:archive', sourceSize: 10, password: 'bad\0password' })).toBe(false)
  })
})
