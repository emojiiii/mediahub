// @vitest-environment node

import JSZip from 'jszip'
import { describe, expect, it } from 'vitest'

import { createStoredArchiveFolderZip } from './ArchiveFolderZip'

describe('ArchiveFolderZip', () => {
  it('creates a valid stored ZIP containing only selected folder descendants', async () => {
    const data = await createStoredArchiveFolderZip([
      { path: 'docs/readme.txt', data: new TextEncoder().encode('read me') },
      { path: 'docs/guides/start.md', data: new TextEncoder().encode('start') },
    ], 'docs')

    expect([...data.slice(0, 4)]).toEqual([0x50, 0x4b, 0x03, 0x04])
    expect(data.byteLength).toBeLessThan(2048)

    const archive = await JSZip.loadAsync(data)
    const files = Object.values(archive.files).filter((entry) => !entry.dir)
    expect(files.map((entry) => entry.name).sort()).toEqual([
      'docs/guides/start.md',
      'docs/readme.txt',
    ])
    expect(await archive.file('docs/readme.txt')?.async('string')).toBe('read me')
    expect(await archive.file('docs/guides/start.md')?.async('string')).toBe('start')
  })

  it('rejects entries outside the selected folder', async () => {
    await expect(createStoredArchiveFolderZip([
      { path: 'root.txt', data: new Uint8Array([1]) },
    ], 'docs')).rejects.toThrow('outside the selected folder')
  })
})
