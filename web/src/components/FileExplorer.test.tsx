import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { api, type ObjectItem } from '../api'
import FileExplorer, { classifyMimeType, folderName } from './FileExplorer'

class TestIntersectionObserver implements IntersectionObserver {
  readonly root: Element | Document | null
  readonly rootMargin: string
  readonly thresholds: readonly number[]
  private readonly callback: IntersectionObserverCallback
  private readonly targets = new Set<Element>()

  constructor(callback: IntersectionObserverCallback, options: IntersectionObserverInit = {}) {
    this.callback = callback
    this.root = options.root ?? null
    this.rootMargin = options.rootMargin ?? '0px'
    this.thresholds = typeof options.threshold === 'number' ? [options.threshold] : options.threshold ?? [0]
    intersectionObservers.push(this)
  }

  observe(target: Element) { this.targets.add(target) }
  unobserve(target: Element) { this.targets.delete(target) }
  disconnect() { this.targets.clear() }
  takeRecords(): IntersectionObserverEntry[] { return [] }
  get hasTargets() { return this.targets.size > 0 }

  trigger(isIntersecting = true) {
    const entries = [...this.targets].map((target): IntersectionObserverEntry => ({
      boundingClientRect: target.getBoundingClientRect(),
      intersectionRatio: isIntersecting ? 1 : 0,
      intersectionRect: target.getBoundingClientRect(),
      isIntersecting,
      rootBounds: null,
      target,
      time: 0,
    }))
    this.callback(entries, this)
  }
}

const intersectionObservers: TestIntersectionObserver[] = []

beforeEach(() => {
  intersectionObservers.length = 0
  vi.stubGlobal('IntersectionObserver', TestIntersectionObserver)
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const IMAGE: ObjectItem = {
  id: 'image-1',
  name: 'sunset.png',
  key: 'photos/sunset.png',
  bucket: 'assets',
  bucketId: 'bucket-1',
  type: 'image/png',
  size: 2048,
  sha256: 'abc',
  revision: 1,
  createdAt: '2026-08-08T00:00:00Z',
  status: 'active',
  visibility: '公开',
}

const PDF: ObjectItem = {
  ...IMAGE,
  id: 'pdf-1',
  name: 'guide.pdf',
  key: 'photos/guide.pdf',
  type: 'application/pdf',
}

const SVG: ObjectItem = {
  ...IMAGE,
  id: 'svg-1',
  name: 'vector.svg',
  key: 'photos/vector.svg',
  type: 'image/svg+xml',
}

const THUMBNAIL_VARIANT_PARAMS = {
  width: 384,
  height: 384,
  fit: 'inside',
  quality: 68,
  format: 'webp',
  blur: 0,
  crop: 'center',
  background: 'ffffff',
} as const

function renderExplorer(
  overrides: Partial<React.ComponentProps<typeof FileExplorer>> = {},
  queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } }),
) {
  const props: React.ComponentProps<typeof FileExplorer> = {
    items: [IMAGE, PDF],
    prefixes: ['photos/trips/'],
    currentPrefix: 'photos/',
    selectedIds: [],
    deletingId: null,
    onOpenFolder: vi.fn(),
    onSelectionChange: vi.fn(),
    onPreview: vi.fn(),
    onOpenDetails: vi.fn(),
    onEdit: vi.fn(),
    onDelete: vi.fn(),
    ...overrides,
  }
  render(<QueryClientProvider client={queryClient}><FileExplorer {...props} /></QueryClientProvider>)
  return props
}

function revealImageThumbnail() {
  const observer = intersectionObservers.find((candidate) => candidate.rootMargin === '240px 0px' && candidate.hasTargets)
  if (!observer) throw new Error('Image thumbnail observer was not registered')
  act(() => observer.trigger())
  return observer
}

describe('FileExplorer helpers', () => {
  it('derives direct folder names from common prefixes', () => {
    expect(folderName('photos/trips/', 'photos/')).toBe('trips')
    expect(folderName('/archive/2026/')).toBe('archive')
    expect(folderName('standalone/')).toBe('standalone')
  })

  it('classifies common MIME families and parameters', () => {
    expect(classifyMimeType('image/avif')).toBe('image')
    expect(classifyMimeType('video/mp4; codecs=avc1')).toBe('video')
    expect(classifyMimeType('application/vnd.openxmlformats-officedocument.spreadsheetml.sheet')).toBe('spreadsheet')
    expect(classifyMimeType('application/vnd.openxmlformats-officedocument.presentationml.presentation')).toBe('presentation')
    expect(classifyMimeType('application/json')).toBe('code')
    expect(classifyMimeType('application/octet-stream')).toBe('other')
  })
})

describe('FileExplorer interactions', () => {
  it('opens folders with double click and Enter', () => {
    const props = renderExplorer()
    const folder = screen.getByRole('option', { name: '文件夹 trips' })

    fireEvent.doubleClick(folder)
    fireEvent.keyDown(folder, { key: 'Enter' })

    expect(props.onOpenFolder).toHaveBeenNthCalledWith(1, 'photos/trips/')
    expect(props.onOpenFolder).toHaveBeenNthCalledWith(2, 'photos/trips/')
  })

  it('selects on click, toggles with modifiers, and previews with keyboard or double click', () => {
    const onSelectionChange = vi.fn()
    const onPreview = vi.fn()
    const props = renderExplorer({ selectedIds: ['pdf-1'], onSelectionChange, onPreview })
    const image = screen.getByRole('option', { name: '文件 sunset.png' })

    fireEvent.click(image)
    expect(onSelectionChange).toHaveBeenLastCalledWith(['image-1'])

    fireEvent.click(image, { ctrlKey: true })
    expect(onSelectionChange).toHaveBeenLastCalledWith(['pdf-1', 'image-1'])

    fireEvent.keyDown(image, { key: ' ' })
    expect(onSelectionChange).toHaveBeenLastCalledWith(['pdf-1', 'image-1'])

    fireEvent.keyDown(image, { key: 'Enter' })
    fireEvent.doubleClick(image)
    expect(onPreview).toHaveBeenNthCalledWith(1, IMAGE)
    expect(onPreview).toHaveBeenNthCalledWith(2, IMAGE)
    expect(props.items).toHaveLength(2)
  })

  it('exposes file actions through the context menu', () => {
    const props = renderExplorer()
    const image = screen.getByRole('option', { name: '文件 sunset.png' })

    fireEvent.contextMenu(image, { clientX: 80, clientY: 90 })
    expect(screen.getByRole('menu', { name: 'sunset.png 操作菜单' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('menuitem', { name: '查看详情' }))
    expect(props.onOpenDetails).toHaveBeenCalledWith(IMAGE)

    fireEvent.contextMenu(image)
    fireEvent.click(screen.getByRole('menuitem', { name: '编辑' }))
    expect(props.onEdit).toHaveBeenCalledWith(IMAGE)

    fireEvent.contextMenu(image)
    fireEvent.click(screen.getByRole('menuitem', { name: '删除' }))
    expect(props.onDelete).toHaveBeenCalledWith(IMAGE)
  })

  it('closes the context menu on outside interaction and Escape', () => {
    renderExplorer()
    const image = screen.getByRole('option', { name: '文件 sunset.png' })

    fireEvent.contextMenu(image)
    fireEvent.pointerDown(document.body)
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()

    fireEvent.contextMenu(image)
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
  })

  it('disables deletion while the object is being deleted', () => {
    renderExplorer({ deletingId: IMAGE.id })
    fireEvent.contextMenu(screen.getByRole('option', { name: '文件 sunset.png' }))

    expect(screen.getByRole('menuitem', { name: '正在删除' })).toBeDisabled()
    expect(screen.getByRole('option', { name: '文件 sunset.png' })).toHaveAttribute('aria-busy', 'true')
  })
})

describe('FileExplorer image thumbnails', () => {
  it('requests a normalized low-cost raster variant only after the image card nears the viewport', async () => {
    const getVariantUrl = vi.spyOn(api, 'getVariantUrl').mockResolvedValue({ url: 'https://cdn.example.test/sunset-thumbnail.webp', expiresAt: '' })
    const getPublicUrl = vi.spyOn(api, 'getPublicUrl')
    const getSignedUrl = vi.spyOn(api, 'getSignedUrl')
    renderExplorer({ items: [IMAGE], prefixes: [] })

    expect(getVariantUrl).not.toHaveBeenCalled()
    expect(screen.getByTestId('thumbnail-loading-image-1')).toBeInTheDocument()

    revealImageThumbnail()
    await waitFor(() => expect(getVariantUrl).toHaveBeenCalledTimes(1))
    expect(getVariantUrl).toHaveBeenCalledWith(IMAGE.id, THUMBNAIL_VARIANT_PARAMS)
    expect(getPublicUrl).not.toHaveBeenCalled()
    expect(getSignedUrl).not.toHaveBeenCalled()
  })

  it('reuses the in-memory Query cache without fetching or rendering an offscreen URL', async () => {
    const getVariantUrl = vi.spyOn(api, 'getVariantUrl').mockResolvedValue({ url: 'https://cdn.example.test/cached.webp', expiresAt: '' })
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    renderExplorer({ items: [IMAGE], prefixes: [] }, queryClient)
    revealImageThumbnail()
    await screen.findByRole('img', { name: 'sunset.png 缩略图' })
    expect(getVariantUrl).toHaveBeenCalledTimes(1)

    cleanup()
    renderExplorer({ items: [IMAGE], prefixes: [] }, queryClient)
    expect(screen.queryByRole('img', { name: 'sunset.png 缩略图' })).not.toBeInTheDocument()
    expect(getVariantUrl).toHaveBeenCalledTimes(1)

    revealImageThumbnail()
    await screen.findByRole('img', { name: 'sunset.png 缩略图' })
    expect(getVariantUrl).toHaveBeenCalledTimes(1)
  })

  it('shows the resolved private raster variant and keeps its URL out of title attributes', async () => {
    const variantUrl = 'https://cdn.example.test/private-thumbnail.webp?signature=secret'
    const getVariantUrl = vi.spyOn(api, 'getVariantUrl').mockResolvedValue({ url: variantUrl, expiresAt: '2026-08-08T00:15:00Z' })
    const getPublicUrl = vi.spyOn(api, 'getPublicUrl')
    const getSignedUrl = vi.spyOn(api, 'getSignedUrl')
    renderExplorer({ items: [{ ...IMAGE, visibility: '私有' }], prefixes: [] })

    revealImageThumbnail()
    const image = await screen.findByRole('img', { name: 'sunset.png 缩略图' })
    expect(image).toHaveAttribute('src', variantUrl)
    expect(image).not.toHaveAttribute('title')
    fireEvent.load(image)

    expect(image).toHaveClass('opacity-100')
    expect(screen.queryByTestId('thumbnail-loading-image-1')).not.toBeInTheDocument()
    expect(getVariantUrl).toHaveBeenCalledWith(IMAGE.id, THUMBNAIL_VARIANT_PARAMS)
    expect(getPublicUrl).not.toHaveBeenCalled()
    expect(getSignedUrl).not.toHaveBeenCalled()
  })

  it('falls back to the image type icon when image decoding fails', async () => {
    vi.spyOn(api, 'getVariantUrl').mockResolvedValue({ url: 'https://cdn.example.test/broken.webp', expiresAt: '' })
    renderExplorer({ items: [IMAGE], prefixes: [] })

    revealImageThumbnail()
    const image = await screen.findByRole('img', { name: 'sunset.png 缩略图' })
    fireEvent.error(image)

    expect(screen.getByTestId('thumbnail-fallback-image-1')).toBeInTheDocument()
    expect(screen.queryByRole('img', { name: 'sunset.png 缩略图' })).not.toBeInTheDocument()
  })

  it('falls back when the URL API returns no usable URL', async () => {
    vi.spyOn(api, 'getVariantUrl').mockResolvedValue({ url: '   ', expiresAt: '' })
    renderExplorer({ items: [IMAGE], prefixes: [] })

    revealImageThumbnail()

    expect(await screen.findByTestId('thumbnail-fallback-image-1')).toBeInTheDocument()
    expect(screen.queryByRole('img')).not.toBeInTheDocument()
  })

  it('uses original public or signed URLs for image MIME types unsupported by variants', async () => {
    const getVariantUrl = vi.spyOn(api, 'getVariantUrl')
    const getPublicUrl = vi.spyOn(api, 'getPublicUrl').mockResolvedValue({ url: 'https://cdn.example.test/vector.svg', expiresAt: '' })
    const getSignedUrl = vi.spyOn(api, 'getSignedUrl').mockResolvedValue({ url: 'https://cdn.example.test/private-vector.svg?signature=secret', expiresAt: '2026-08-08T00:15:00Z' })
    renderExplorer({ items: [SVG], prefixes: [] })

    revealImageThumbnail()
    await screen.findByRole('img', { name: 'vector.svg 缩略图' })
    expect(getPublicUrl).toHaveBeenCalledWith(SVG.id)
    expect(getVariantUrl).not.toHaveBeenCalled()

    cleanup()
    const privateSvg = { ...SVG, id: 'svg-private', visibility: '私有' as const }
    renderExplorer({ items: [privateSvg], prefixes: [] })
    revealImageThumbnail()
    await screen.findByRole('img', { name: 'vector.svg 缩略图' })
    expect(getSignedUrl).toHaveBeenCalledWith(privateSvg.id)
    expect(getVariantUrl).not.toHaveBeenCalled()
  })

  it('does not create thumbnail URL requests for non-image files', () => {
    const getVariantUrl = vi.spyOn(api, 'getVariantUrl')
    const getPublicUrl = vi.spyOn(api, 'getPublicUrl')
    const getSignedUrl = vi.spyOn(api, 'getSignedUrl')
    renderExplorer({ items: [PDF], prefixes: [] })

    for (const observer of intersectionObservers) act(() => observer.trigger())
    expect(getVariantUrl).not.toHaveBeenCalled()
    expect(getPublicUrl).not.toHaveBeenCalled()
    expect(getSignedUrl).not.toHaveBeenCalled()
    expect(screen.queryByTestId(/thumbnail-loading/)).not.toBeInTheDocument()
  })

  it('disconnects an untriggered observer when the explorer unmounts', () => {
    const getVariantUrl = vi.spyOn(api, 'getVariantUrl')
    renderExplorer({ items: [IMAGE], prefixes: [] })
    const observer = intersectionObservers.find((candidate) => candidate.hasTargets)

    expect(observer?.hasTargets).toBe(true)
    cleanup()

    expect(observer?.hasTargets).toBe(false)
    expect(getVariantUrl).not.toHaveBeenCalled()
  })
})
