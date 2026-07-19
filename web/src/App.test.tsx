import { useState } from 'react'
import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'

import {
  AccessKeyEditor,
  DEFAULT_OBJECT_FILTERS,
  ObjectPagination,
  OneTimeSecretPanel,
  bucketObjectPath,
  buildUploadObjectKey,
  directoryBreadcrumbs,
  normalizeDirectoryPrefix,
  normalizeUploadPath,
  objectListRefetchInterval,
  removeObjectIdsFromPages,
  uploadObjectKeyValidationError,
  uploadPathValidationError,
} from './App'
import type { ObjectItem } from './api'

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

const OBJECT: ObjectItem = {
  id: 'media_test',
  name: 'sample.png',
  key: 'images/sample.png',
  bucket: 'images',
  bucketId: 'bucket_images',
  type: 'image/png',
  size: 128,
  sha256: 'abc123',
  revision: 1,
  createdAt: '2026-07-19T00:00:00.000Z',
  status: 'active',
  visibility: '私有',
}

describe('console helpers and standalone controls', () => {
  it('builds an encoded Bucket object path prefix', () => {
    expect(bucketObjectPath('app demo', 'media assets')).toBe('/app%20demo/media%20assets/')
  })

  it('normalizes upload paths and builds bounded Object Keys', () => {
    expect(normalizeUploadPath(' /自定义路径//images\\2026/ ')).toBe('自定义路径/images/2026')
    expect(buildUploadObjectKey('/自定义路径/images/', 'demo.png')).toBe('自定义路径/images/demo.png')
    expect(uploadPathValidationError('../outside')).toBe('路径不能包含 . 或 .. 段')
    expect(uploadObjectKeyValidationError(`${'a'.repeat(1020)}/file.png`)).toBe('Object Key 不能超过 1024 字节')
  })

  it('normalizes virtual directory paths and builds breadcrumb targets', () => {
    expect(normalizeDirectoryPrefix(' /images//avatar\\2026/ ')).toBe('images/avatar/2026/')
    expect(normalizeDirectoryPrefix('/')).toBe('')
    expect(directoryBreadcrumbs('images/avatar/')).toEqual([
      { label: 'images', prefix: 'images/' },
      { label: 'avatar', prefix: 'images/avatar/' },
    ])
  })

  it('keeps cursor pagination and page size together in the result footer', async () => {
    const user = userEvent.setup()
    const onNext = vi.fn()
    const onPageSizeChange = vi.fn()
    const onPrevious = vi.fn()
    render(<ObjectPagination currentPage={2} fetching={false} hasNext hasPrevious itemCount={25} pageSize={50} onNext={onNext} onPageSizeChange={onPageSizeChange} onPrevious={onPrevious} />)

    expect(screen.getByText('本页 25 项')).toBeInTheDocument()
    expect(screen.getByText('第 2 页')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '上一页' }))
    await user.click(screen.getByRole('button', { name: '下一页' }))
    expect(onPrevious).toHaveBeenCalledOnce()
    expect(onNext).toHaveBeenCalledOnce()

    await user.click(screen.getByRole('button', { name: '每页数量' }))
    await user.click(await screen.findByRole('menuitemradio', { name: '100' }))
    expect(onPageSizeChange).toHaveBeenCalledWith(100)
  })

  it('keeps delete-pending refresh and cache removal behavior deterministic', () => {
    expect(DEFAULT_OBJECT_FILTERS).toEqual({ limit: 25, status: 'active' })
    expect(objectListRefetchInterval({ pages: [{ items: [{ status: 'active' }] }] })).toBe(false)
    expect(objectListRefetchInterval({ pages: [{ items: [{ status: 'delete_pending' }] }] })).toBe(2000)

    const filtered = removeObjectIdsFromPages(
      { pages: [{ items: [OBJECT], commonPrefixes: [], nextCursor: null }], pageParams: [''] },
      [OBJECT.id],
    )
    expect(filtered?.pages[0]?.items).toEqual([])
  })

  it('submits the selected access-key permissions', async () => {
    const user = userEvent.setup()
    const onSave = vi.fn()
    render(<AccessKeyEditor accessKey={null} pending={false} error={null} onClose={vi.fn()} onSave={onSave} />)
    await user.type(screen.getByRole('textbox', { name: '名称' }), 'reader')
    expect(screen.getByRole('checkbox', { name: 'media:read' })).toBeChecked()
    await user.click(screen.getByRole('checkbox', { name: 'bucket:list' }))
    await user.click(screen.getByRole('button', { name: '创建密钥' }))
    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ name: 'reader', permissions: ['media:read', 'bucket:list'] }))
  })

  it('discards a one-time secret after the panel closes', async () => {
    const user = userEvent.setup()
    function Harness() {
      const [visible, setVisible] = useState(true)
      return visible ? <OneTimeSecretPanel value={{ title: 'Secret 已创建', identifier: 'secret-id', secret: 'only-once-value' }} onClose={() => setVisible(false)} /> : null
    }
    render(<Harness />)
    expect(screen.getByDisplayValue('only-once-value')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '关闭并丢弃 Secret' }))
    expect(screen.queryByDisplayValue('only-once-value')).not.toBeInTheDocument()
  })
})
