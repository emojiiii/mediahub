import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { DEFAULT_VARIANT_PARAMS, ObjectPreviewModal, isRasterImageMimeType, isValidVariantParams } from './App'
import { api, type ObjectItem } from './api'

const IMAGE: ObjectItem = {
  id: 'media_image',
  name: 'sample.png',
  key: 'previews/sample.png',
  bucket: 'images',
  bucketId: 'bucket_images',
  type: 'image/png',
  size: 2_048,
  sha256: 'abc123',
  revision: 1,
  createdAt: '2026-07-18T08:00:00.000Z',
  status: 'active',
  visibility: '私有',
}

function renderPreview(item: ObjectItem = IMAGE) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(<QueryClientProvider client={queryClient}><ObjectPreviewModal item={item} onClose={vi.fn()} /></QueryClientProvider>)
}

async function advanceTimers(milliseconds: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(milliseconds)
  })
}

afterEach(() => {
  cleanup()
  vi.clearAllTimers()
  vi.useRealTimers()
  vi.restoreAllMocks()
})

describe('图片实时 Variant 预览', () => {
  it('只将明确的栅格图片 MIME 交给专用舞台', () => {
    expect(isRasterImageMimeType('image/png')).toBe(true)
    expect(isRasterImageMimeType('IMAGE/WEBP; charset=binary')).toBe(true)
    expect(isRasterImageMimeType('image/avif')).toBe(false)
    expect(isRasterImageMimeType('image/heif')).toBe(false)
    expect(isRasterImageMimeType('image/svg+xml')).toBe(false)
    expect(isRasterImageMimeType('text/xml')).toBe(false)
  })

  it('完整校验实时 Variant 参数边界', () => {
    expect(isValidVariantParams(DEFAULT_VARIANT_PARAMS)).toBe(true)
    expect(isValidVariantParams({ ...DEFAULT_VARIANT_PARAMS, width: 0 })).toBe(false)
    expect(isValidVariantParams({ ...DEFAULT_VARIANT_PARAMS, height: 4097 })).toBe(false)
    expect(isValidVariantParams({ ...DEFAULT_VARIANT_PARAMS, quality: 101 })).toBe(false)
    expect(isValidVariantParams({ ...DEFAULT_VARIANT_PARAMS, blur: -1 })).toBe(false)
    expect(isValidVariantParams({ ...DEFAULT_VARIANT_PARAMS, fit: 'stretch' as typeof DEFAULT_VARIANT_PARAMS.fit })).toBe(false)
    expect(isValidVariantParams({ ...DEFAULT_VARIANT_PARAMS, format: 'gif' as typeof DEFAULT_VARIANT_PARAMS.format })).toBe(false)
    expect(isValidVariantParams({ ...DEFAULT_VARIANT_PARAMS, format: 'avif' as typeof DEFAULT_VARIANT_PARAMS.format })).toBe(false)
    expect(isValidVariantParams({ ...DEFAULT_VARIANT_PARAMS, crop: 'face' as typeof DEFAULT_VARIANT_PARAMS.crop })).toBe(false)
    expect(isValidVariantParams({ ...DEFAULT_VARIANT_PARAMS, background: 'white' })).toBe(false)
  })

  it('让完整预览区域成为 object-contain 的稳定尺寸框', async () => {
    vi.spyOn(api, 'getSignedUrl').mockResolvedValue({
      url: 'https://media.example.test/original.png',
      expiresAt: '2026-07-18T09:00:00.000Z',
    })

    renderPreview()

    expect(await screen.findByTestId('raster-preview-image')).toHaveClass(
      'h-full',
      'min-h-0',
      'w-full',
      'min-w-0',
      'object-contain',
      'object-center',
    )
    expect(screen.getByTestId('raster-preview-viewport')).toHaveClass(
      'min-h-0',
      'min-w-0',
      'flex-1',
      'overflow-hidden',
    )
  })

  it('防抖请求 Variant，并在新图加载前保留当前图片与打开链接', async () => {
    const original = 'https://media.example.test/original.png?token=original'
    vi.spyOn(api, 'getSignedUrl').mockResolvedValue({ url: original, expiresAt: '2026-07-18T09:00:00.000Z' })
    const getVariantUrl = vi.spyOn(api, 'getVariantUrl').mockImplementation(async (_mediaId, params) => ({
      url: `https://media.example.test/variant-${params.width}.${params.format}?quality=${params.quality}`,
      expiresAt: '2026-07-18T09:05:00.000Z',
    }))

    renderPreview()
    const visibleImage = await screen.findByTestId('raster-preview-image')
    expect(visibleImage).toHaveAttribute('src', original)
    expect(screen.queryByTestId('open-file-viewer')).not.toBeInTheDocument()
    expect(screen.getByRole('link', { name: '在新窗口打开' })).toHaveAttribute('href', original)

    vi.useFakeTimers()
    fireEvent.click(screen.getByRole('button', { name: 'Variant' }))
    await advanceTimers(349)
    expect(getVariantUrl).not.toHaveBeenCalled()
    await advanceTimers(1)
    await advanceTimers(1)
    expect(getVariantUrl).toHaveBeenCalledWith(IMAGE.id, DEFAULT_VARIANT_PARAMS)

    const firstPreloader = screen.getByTestId('variant-image-preloader')
    const firstVariantUrl = 'https://media.example.test/variant-600.webp?quality=80'
    expect(firstPreloader).toHaveAttribute('src', firstVariantUrl)
    expect(visibleImage).toHaveAttribute('src', original)
    expect(screen.getByRole('link', { name: '在新窗口打开' })).toHaveAttribute('href', original)
    fireEvent.load(firstPreloader)
    expect(visibleImage).toHaveAttribute('src', firstVariantUrl)
    expect(screen.getByRole('link', { name: '在新窗口打开' })).toHaveAttribute('href', firstVariantUrl)

    fireEvent.change(screen.getByRole('spinbutton', { name: 'Variant 宽度' }), { target: { value: '720' } })
    await advanceTimers(349)
    expect(getVariantUrl).toHaveBeenCalledTimes(1)
    expect(visibleImage).toHaveAttribute('src', firstVariantUrl)
    await advanceTimers(1)
    await advanceTimers(1)

    const secondPreloader = screen.getByTestId('variant-image-preloader')
    const secondVariantUrl = 'https://media.example.test/variant-720.webp?quality=80'
    expect(secondPreloader).toHaveAttribute('src', secondVariantUrl)
    expect(visibleImage).toHaveAttribute('src', firstVariantUrl)
    expect(screen.getByRole('link', { name: '在新窗口打开' })).toHaveAttribute('href', firstVariantUrl)
    expect(screen.getByText('720 × 600 · cover · webp · Q80 · Blur 0')).toBeInTheDocument()
    fireEvent.load(secondPreloader)
    expect(visibleImage).toHaveAttribute('src', secondVariantUrl)
    expect(screen.getByRole('link', { name: '在新窗口打开' })).toHaveAttribute('href', secondVariantUrl)
  })

  it('参数无效时不请求 Variant', async () => {
    vi.spyOn(api, 'getSignedUrl').mockResolvedValue({ url: 'https://media.example.test/original.png', expiresAt: '2026-07-18T09:00:00.000Z' })
    const getVariantUrl = vi.spyOn(api, 'getVariantUrl')
    renderPreview()
    await screen.findByTestId('raster-preview-image')

    vi.useFakeTimers()
    fireEvent.change(screen.getByRole('spinbutton', { name: 'Variant 宽度' }), { target: { value: '0' } })
    await advanceTimers(500)
    expect(getVariantUrl).not.toHaveBeenCalled()
    expect(screen.getByText('参数无效')).toBeInTheDocument()
  })

  it('SVG 继续走通用安全查看器且不显示 Variant 参数栏', async () => {
    vi.spyOn(api, 'getSignedUrl').mockResolvedValue({ url: 'https://media.example.test/vector.svg', expiresAt: '2026-07-18T09:00:00.000Z' })
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response('<svg xmlns="http://www.w3.org/2000/svg"/>', {
      headers: { 'Content-Length': '41', 'Content-Type': 'image/svg+xml' },
    }))
    const getVariantUrl = vi.spyOn(api, 'getVariantUrl')
    renderPreview({ ...IMAGE, id: 'media_svg', name: 'vector.svg', key: 'vector.svg', type: 'image/svg+xml' })

    expect(await screen.findByTestId('open-file-viewer')).toBeInTheDocument()
    expect(screen.queryByTestId('image-variant-toolbar')).not.toBeInTheDocument()
    expect(getVariantUrl).not.toHaveBeenCalled()
  })
})
