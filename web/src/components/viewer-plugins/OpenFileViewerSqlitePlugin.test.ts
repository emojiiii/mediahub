import { describe, expect, it } from 'vitest'

import { isSqliteFile } from './OpenFileViewerSqlitePlugin'

describe('isSqliteFile', () => {
  it.each([
    { extension: 'sqlite', mimeType: 'application/octet-stream' },
    { extension: '.SQLITE3', mimeType: 'application/octet-stream' },
    { extension: 'unknown', mimeType: 'application/vnd.sqlite3; charset=binary' },
    { extension: 'db', mimeType: '' },
  ])('matches SQLite files by extension or MIME', (file) => {
    expect(isSqliteFile(file)).toBe(true)
  })

  it('does not claim unrelated generic binary files', () => {
    expect(isSqliteFile({ extension: 'bin', mimeType: 'application/octet-stream' })).toBe(false)
  })
})
