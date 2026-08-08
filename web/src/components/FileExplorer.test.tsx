import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import type { ObjectItem } from '../api'
import FileExplorer, { classifyMimeType, folderName } from './FileExplorer'

afterEach(cleanup)

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

function renderExplorer(overrides: Partial<React.ComponentProps<typeof FileExplorer>> = {}) {
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
  render(<FileExplorer {...props} />)
  return props
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
