import { beforeEach, describe, expect, it } from 'vitest'

import { applyPageMetadata } from './App'

describe('applyPageMetadata', () => {
  beforeEach(() => {
    document.head.innerHTML = '<meta name="description" content=""><meta name="robots" content="">'
  })

  it('publishes canonical and social metadata for the public landing page', () => {
    applyPageMetadata('/')

    expect(document.title).toContain('PrismArk')
    expect(document.querySelector<HTMLLinkElement>('link[rel="canonical"]')?.href).toBe(
      `${window.location.origin}/`,
    )
    expect(document.querySelector<HTMLMetaElement>('meta[name="robots"]')?.content).toBe(
      'index,follow,max-image-preview:large',
    )
    expect(document.querySelector<HTMLMetaElement>('meta[property="og:url"]')?.content).toBe(
      `${window.location.origin}/`,
    )
    expect(document.querySelector<HTMLMetaElement>('meta[property="og:image"]')?.content).toBe(
      `${window.location.origin}/brand/prismark-mark-512.png`,
    )
  })

  it('marks authentication and console pages as private', () => {
    applyPageMetadata('/console')

    expect(document.querySelector<HTMLMetaElement>('meta[name="robots"]')?.content).toBe(
      'noindex,nofollow,noarchive',
    )
    expect(document.querySelector<HTMLLinkElement>('link[rel="canonical"]')?.href).toBe(
      `${window.location.origin}/console`,
    )
  })

  it('keeps exactly one canonical element across route changes', () => {
    document.head.insertAdjacentHTML(
      'beforeend',
      '<link rel="canonical" href="https://invalid.example/"><link rel="canonical" href="https://duplicate.example/">',
    )

    applyPageMetadata('/')

    expect(document.querySelectorAll('link[rel="canonical"]')).toHaveLength(1)
  })
})
