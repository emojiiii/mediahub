// @vitest-environment node

import { describe, expect, it } from 'vitest'

import { parseSevenZipListOutput } from './SevenZipArchive'

const encryptedListing = [
  'Listing archive: /mediahub-archive',
  '',
  '----------',
  'Path = docs',
  'Size = 0',
  'Attributes = D drwxrwxrwx',
  'Encrypted = -',
  '',
  'Path = docs/readme.txt',
  'Size = 28',
  'Attributes = A -rwxrwxrwx',
  'Encrypted = +',
  '',
]

describe('SevenZipArchive', () => {
  it('parses the technical listing into safe encrypted entries', () => {
    expect(parseSevenZipListOutput(encryptedListing)).toEqual({
      encrypted: true,
      entries: [
        { path: 'docs', size: 0, directory: true },
        { path: 'docs/readme.txt', size: 28, directory: false },
      ],
    })
  })

  it('accepts Folder fields and rejects traversal, links, and invalid sizes', () => {
    expect(parseSevenZipListOutput([
      '----------',
      'Path = nested',
      'Folder = +',
      'Size = 0',
      '',
    ]).entries).toEqual([{ path: 'nested', size: 0, directory: true }])

    expect(() => parseSevenZipListOutput([
      '----------',
      'Path = ../outside.txt',
      'Size = 1',
      '',
    ])).toThrow('越过目录边界')
    expect(() => parseSevenZipListOutput([
      '----------',
      'Path = link',
      'Size = 1',
      'Symbolic Link = target',
      '',
    ])).toThrow('link entry')
    expect(() => parseSevenZipListOutput([
      '----------',
      'Path = invalid.txt',
      'Size = 1.5',
      '',
    ])).toThrow('invalid entry size')
  })
})
