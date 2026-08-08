import { describe, expect, it } from 'vitest'

import { consoleNavGroups, consoleNavItem, settingsNavItem } from './console-navigation'

describe('console navigation registry', () => {
  it('keeps route paths unique', () => {
    const paths = [...consoleNavGroups.flatMap((group) => group.items), settingsNavItem].map((item) => item.path)
    expect(new Set(paths).size).toBe(paths.length)
  })

  it('marks backend-dependent S3 governance routes as planned', () => {
    for (const path of ['policies', 'versioning', 'lifecycle', 'object-lock']) {
      expect(consoleNavItem(path)?.state).toBe('planned')
    }
    expect(consoleNavItem('objects')?.state).toBeUndefined()
    expect(consoleNavItem('previews')?.state).toBeUndefined()
  })
})
