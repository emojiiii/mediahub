import { describe, expect, it } from 'vitest'

import { archiveFolderPaths, buildArchiveTree } from './ArchiveTree'

describe('ArchiveTree', () => {
  it('synthesizes missing parent directories and sorts folders before files', () => {
    const tree = buildArchiveTree([
      { path: 'root.txt', size: 1, directory: false },
      { path: 'docs/readme.txt', size: 2, directory: false },
      { path: 'docs/guides/start.md', size: 3, directory: false },
    ])

    expect(tree.map((node) => node.path)).toEqual(['docs', 'root.txt'])
    expect(tree[0].children.map((node) => node.path)).toEqual(['docs/guides', 'docs/readme.txt'])
    expect(archiveFolderPaths(tree)).toEqual(['docs', 'docs/guides'])
  })
})
