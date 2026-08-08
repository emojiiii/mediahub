import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { useState } from 'react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import {
  DEFAULT_VARIANT_PARAMS,
  ObjectPreviewModal,
  RASTER_PREVIEW_MAX_ZOOM,
  RASTER_PREVIEW_MIN_ZOOM,
  calculateRasterPreviewFitZoom,
  clampRasterPreviewZoom,
  isRasterImageMimeType,
  isValidVariantParams,
} from './App'
import { api, type ObjectItem } from './api'
import { ThemeProvider } from './theme'

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
  const preview = (nextItem: ObjectItem) => <ThemeProvider defaultTheme="light"><QueryClientProvider client={queryClient}><ObjectPreviewModal item={nextItem} onClose={vi.fn()} /></QueryClientProvider></ThemeProvider>
  const rendered = render(preview(item))
  return { ...rendered, rerenderPreview: (nextItem: ObjectItem) => rendered.rerender(preview(nextItem)) }
}

const PREVIEW_ITEMS: ObjectItem[] = [
  IMAGE,
  { ...IMAGE, id: 'media_second', name: 'second.png', key: 'previews/second.png', sha256: 'def456', revision: 2 },
  { ...IMAGE, id: 'media_third', name: 'third.png', key: 'previews/third.png', sha256: 'ghi789', revision: 3 },
]

function renderPreviewBrowser(initialIndex = 0) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  function PreviewBrowser() {
    const [index, setIndex] = useState(initialIndex)
    const item = PREVIEW_ITEMS[index]
    return <ThemeProvider defaultTheme="light"><QueryClientProvider client={queryClient}><ObjectPreviewModal
      item={item}
      previousItem={index > 0 ? PREVIEW_ITEMS[index - 1] : undefined}
      nextItem={index < PREVIEW_ITEMS.length - 1 ? PREVIEW_ITEMS[index + 1] : undefined}
      position={index + 1}
      totalItems={PREVIEW_ITEMS.length}
      onNavigate={(nextItem) => setIndex(PREVIEW_ITEMS.findIndex((candidate) => candidate.id === nextItem.id))}
      onClose={vi.fn()}
    /></QueryClientProvider></ThemeProvider>
  }
  return render(<PreviewBrowser />)
}

function loadRasterImage({ imageWidth = 1600, imageHeight = 1200, viewportWidth = 800, viewportHeight = 600 } = {}) {
  const viewport = screen.getByTestId('raster-preview-viewport')
  const image = screen.getByTestId('raster-preview-image')
  Object.defineProperties(viewport, {
    clientWidth: { configurable: true, value: viewportWidth },
    clientHeight: { configurable: true, value: viewportHeight },
  })
  Object.defineProperties(image, {
    naturalWidth: { configurable: true, value: imageWidth },
    naturalHeight: { configurable: true, value: imageHeight },
  })
  fireEvent(window, new Event('resize'))
  fireEvent.load(image)
  return { image, viewport }
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

  it('让完整预览区域成为可缩放且稳定的 object-contain 画布', async () => {
    vi.spyOn(api, 'getSignedUrl').mockResolvedValue({
      url: 'https://media.example.test/original.png',
      expiresAt: '2026-07-18T09:00:00.000Z',
    })

    renderPreview()

    expect(await screen.findByTestId('raster-preview-image')).toHaveClass(
      'absolute',
      'max-h-none',
      'max-w-none',
      'object-contain',
      'object-center',
    )
    expect(screen.getByTestId('raster-preview-viewport')).toHaveClass(
      'min-h-0',
      'min-w-0',
      'flex-1',
      'overflow-hidden',
    )
    expect(screen.getByRole('toolbar', { name: '图片缩放控制' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '适应窗口（0）' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '显示为原始大小 100%' })).toBeInTheDocument()
  })

  it('限制手动缩放边界，同时允许适应窗口低于最小手动缩放值', () => {
    expect(clampRasterPreviewZoom(0)).toBe(RASTER_PREVIEW_MIN_ZOOM)
    expect(clampRasterPreviewZoom(10)).toBe(RASTER_PREVIEW_MAX_ZOOM)
    expect(clampRasterPreviewZoom(Number.NaN)).toBe(1)
    expect(calculateRasterPreviewFitZoom(
      { width: 800, height: 600 },
      { width: 20_000, height: 10_000 },
    )).toBeLessThan(RASTER_PREVIEW_MIN_ZOOM)
  })

  it('按钮缩放会在 10% 到 800% 边界停止', async () => {
    vi.spyOn(api, 'getSignedUrl').mockResolvedValue({ url: 'https://media.example.test/original.png', expiresAt: '' })
    renderPreview()
    await screen.findByTestId('raster-preview-image')
    loadRasterImage({ imageWidth: 400, imageHeight: 300, viewportWidth: 800, viewportHeight: 600 })

    fireEvent.click(screen.getByRole('button', { name: '显示为原始大小 100%' }))
    const zoomIn = screen.getByRole('button', { name: '放大图片（+）' })
    for (let index = 0; index < 20; index += 1) fireEvent.click(zoomIn)
    expect(screen.getByLabelText('当前缩放 800%')).toBeInTheDocument()
    expect(zoomIn).toBeDisabled()

    const zoomOut = screen.getByRole('button', { name: '缩小图片（-）' })
    for (let index = 0; index < 30; index += 1) fireEvent.click(zoomOut)
    expect(screen.getByLabelText('当前缩放 10%')).toBeInTheDocument()
    expect(zoomOut).toBeDisabled()
  })

  it('支持快捷键与双击切换，并忽略文本输入中的按键', async () => {
    vi.spyOn(api, 'getSignedUrl').mockResolvedValue({ url: 'https://media.example.test/original.png', expiresAt: '' })
    renderPreview()
    await screen.findByTestId('raster-preview-image')
    const { viewport } = loadRasterImage()

    expect(viewport).toHaveAttribute('data-view-mode', 'fit')
    fireEvent.keyDown(window, { key: '+' })
    expect(viewport).toHaveAttribute('data-view-mode', 'custom')
    fireEvent.keyDown(window, { key: '0' })
    expect(viewport).toHaveAttribute('data-view-mode', 'fit')

    const widthInput = screen.getByRole('spinbutton', { name: 'Variant 宽度' })
    fireEvent.keyDown(widthInput, { key: '+' })
    expect(viewport).toHaveAttribute('data-view-mode', 'fit')

    fireEvent.doubleClick(viewport)
    expect(viewport).toHaveAttribute('data-view-mode', 'actual')
    fireEvent.doubleClick(viewport)
    expect(viewport).toHaveAttribute('data-view-mode', 'fit')
  })

  it('100% 下可拖拽平移，并在模式或对象切换时重置视图', async () => {
    vi.spyOn(api, 'getSignedUrl').mockImplementation(async (mediaId) => ({ url: `https://media.example.test/${mediaId}.png`, expiresAt: '' }))
    const rendered = renderPreview()
    await screen.findByTestId('raster-preview-image')
    const { image, viewport } = loadRasterImage({ imageWidth: 1000, imageHeight: 800, viewportWidth: 400, viewportHeight: 300 })

    fireEvent.click(screen.getByRole('button', { name: '显示为原始大小 100%' }))
    fireEvent.pointerDown(viewport, { button: 0, pointerId: 7, clientX: 100, clientY: 100 })
    fireEvent.pointerMove(viewport, { pointerId: 7, clientX: 150, clientY: 130 })
    expect(image).toHaveStyle({ transform: 'translate(-50%, -50%) translate(50px, 30px) scale(1)' })
    fireEvent.pointerUp(viewport, { pointerId: 7, clientX: 150, clientY: 130 })

    fireEvent.click(screen.getByRole('button', { name: 'Variant' }))
    await waitFor(() => expect(viewport).toHaveAttribute('data-view-mode', 'fit'))
    expect(image.style.transform).toContain('translate(0px, 0px)')

    fireEvent.click(screen.getByRole('button', { name: '显示为原始大小 100%' }))
    rendered.rerenderPreview({ ...IMAGE, id: 'media_next', name: 'next.png', key: 'previews/next.png', revision: 2 })
    await waitFor(() => expect(screen.getByTestId('raster-preview-viewport')).toHaveAttribute('data-view-mode', 'fit'))
  })

  it('公开对象使用无 token 链接预览，并可复制原链和短链', async () => {
    const publicUrl = 'https://media.example.test/app_test/images/previews/sample.png'
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } })
    const getSignedUrl = vi.spyOn(api, 'getSignedUrl')
    vi.spyOn(api, 'getPublicUrl').mockResolvedValue({ url: publicUrl, expiresAt: '' })
    const createShortLink = vi.spyOn(api, 'createShortLink').mockResolvedValue({
      code: 'sample',
      url: 'https://media.example.test/s/sample',
      targetUrl: publicUrl,
    })

    renderPreview({ ...IMAGE, visibility: '公开' })

    expect(await screen.findByTestId('raster-preview-image')).toHaveAttribute('src', publicUrl)
    expect(getSignedUrl).not.toHaveBeenCalled()
    expect(screen.getByRole('link', { name: '在新窗口打开' })).toHaveAttribute('href', publicUrl)

    fireEvent.click(screen.getByRole('button', { name: '复制公开链接' }))
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(publicUrl))

    fireEvent.click(screen.getByRole('button', { name: '复制短链' }))
    await waitFor(() => expect(createShortLink).toHaveBeenCalledWith(publicUrl))
    await waitFor(() => expect(writeText).toHaveBeenCalledWith('https://media.example.test/s/sample'))
  })

  it('私有对象不提供公开链接复制入口', async () => {
    vi.spyOn(api, 'getSignedUrl').mockResolvedValue({
      url: 'https://media.example.test/private.png?token=secret',
      expiresAt: '2026-07-18T09:00:00.000Z',
    })

    renderPreview()

    await screen.findByTestId('raster-preview-image')
    expect(screen.queryByRole('button', { name: '复制公开链接' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '复制短链' })).not.toBeInTheDocument()
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

describe('对象预览连续浏览', () => {
  it('显示当前位置，并通过边界明确的按钮连续浏览可见文件', async () => {
    vi.spyOn(api, 'getSignedUrl').mockImplementation(async (mediaId) => ({
      url: `https://media.example.test/${mediaId}.png?signature=${mediaId}`,
      expiresAt: '',
    }))

    renderPreviewBrowser()

    expect(await screen.findByTestId('raster-preview-image')).toHaveAttribute('src', expect.stringContaining('media_image'))
    expect(screen.getByLabelText('当前第 1 项，共 3 项')).toHaveTextContent('1/3')
    expect(screen.getByRole('button', { name: '预览上一项' })).toBeDisabled()
    expect(screen.getByRole('button', { name: '预览下一项' })).toBeEnabled()

    fireEvent.click(screen.getByRole('button', { name: '预览下一项' }))
    expect(await screen.findByRole('heading', { name: 'second.png' })).toBeInTheDocument()
    expect(screen.getByLabelText('当前第 2 项，共 3 项')).toHaveTextContent('2/3')
    expect(screen.getByRole('button', { name: '预览上一项' })).toBeEnabled()
    expect(screen.getByRole('button', { name: '预览下一项' })).toBeEnabled()

    fireEvent.click(screen.getByRole('button', { name: '预览下一项' }))
    expect(await screen.findByRole('heading', { name: 'third.png' })).toBeInTheDocument()
    expect(screen.getByLabelText('当前第 3 项，共 3 项')).toHaveTextContent('3/3')
    expect(screen.getByRole('button', { name: '预览下一项' })).toBeDisabled()
  })

  it('支持左右方向键，但不劫持表单、contenteditable 与 Variant 参数区域', async () => {
    vi.spyOn(api, 'getSignedUrl').mockImplementation(async (mediaId) => ({ url: `https://media.example.test/${mediaId}.png`, expiresAt: '' }))
    renderPreviewBrowser(1)
    await screen.findByRole('heading', { name: 'second.png' })

    fireEvent.keyDown(window, { key: 'ArrowRight' })
    expect(await screen.findByRole('heading', { name: 'third.png' })).toBeInTheDocument()
    fireEvent.keyDown(window, { key: 'ArrowLeft' })
    expect(await screen.findByRole('heading', { name: 'second.png' })).toBeInTheDocument()

    fireEvent.keyDown(screen.getByRole('spinbutton', { name: 'Variant 宽度' }), { key: 'ArrowRight' })
    fireEvent.keyDown(screen.getByRole('button', { name: 'Variant 格式' }), { key: 'ArrowLeft' })

    for (const tagName of ['input', 'select', 'textarea']) {
      const control = document.createElement(tagName)
      document.body.appendChild(control)
      fireEvent.keyDown(control, { key: 'ArrowRight' })
      control.remove()
    }
    const editable = document.createElement('div')
    editable.setAttribute('contenteditable', 'true')
    const editableChild = document.createElement('span')
    editable.appendChild(editableChild)
    document.body.appendChild(editable)
    fireEvent.keyDown(editableChild, { key: 'ArrowRight' })
    editable.remove()

    expect(screen.getByRole('heading', { name: 'second.png' })).toBeInTheDocument()
    expect(screen.getByLabelText('当前第 2 项，共 3 项')).toBeInTheDocument()
  })

  it('切换对象时重置原图、Variant、缩放和错误状态，且不沿用旧签名 URL', async () => {
    vi.spyOn(api, 'getSignedUrl').mockImplementation(async (mediaId) => {
      if (mediaId === IMAGE.id) throw new Error('第一项签名已失效')
      return { url: `https://media.example.test/${mediaId}.png?signature=${mediaId}`, expiresAt: '' }
    })

    renderPreviewBrowser()
    expect(await screen.findByText('预览加载失败')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '预览下一项' }))

    const secondImage = await screen.findByTestId('raster-preview-image')
    expect(secondImage).toHaveAttribute('src', expect.stringContaining('signature=media_second'))
    expect(secondImage).not.toHaveAttribute('src', expect.stringContaining('media_image'))
    expect(screen.queryByText('预览加载失败')).not.toBeInTheDocument()

    loadRasterImage()
    fireEvent.change(screen.getByRole('spinbutton', { name: 'Variant 宽度' }), { target: { value: '720' } })
    fireEvent.click(screen.getByRole('button', { name: '显示为原始大小 100%' }))
    expect(screen.getByTestId('raster-preview-viewport')).toHaveAttribute('data-view-mode', 'actual')
    expect(screen.getByRole('spinbutton', { name: 'Variant 宽度' })).toHaveValue(720)

    fireEvent.click(screen.getByRole('button', { name: '预览下一项' }))
    const thirdImage = await screen.findByTestId('raster-preview-image')
    expect(thirdImage).toHaveAttribute('src', expect.stringContaining('signature=media_third'))
    expect(thirdImage).not.toHaveAttribute('src', expect.stringContaining('signature=media_second'))
    expect(screen.getByTestId('raster-preview-viewport')).toHaveAttribute('data-view-mode', 'fit')
    expect(screen.getByRole('spinbutton', { name: 'Variant 宽度' })).toHaveValue(DEFAULT_VARIANT_PARAMS.width)
    expect(screen.getByText('显示原图')).toBeInTheDocument()
  })
})
