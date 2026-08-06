import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { useState } from 'react'
import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'

import {
  AccessKeyEditor,
  AdminSystemVersionPanel,
  DEFAULT_OBJECT_FILTERS,
  ObjectPagination,
  OneTimeSecretPanel,
  bucketObjectPath,
  buildUploadObjectKey,
  directoryBreadcrumbs,
  normalizeDirectoryPrefix,
  normalizeUploadPath,
  ObjectRowActions,
  objectListRefetchInterval,
  removeObjectIdsFromPages,
  uploadObjectKeyValidationError,
  uploadPathValidationError,
} from './App'
import { api, type AdminSystemVersion, type ObjectItem } from './api'

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

const SYSTEM_VERSION: AdminSystemVersion = {
  currentVersion: 'prod-aaaaaaaaaaaa',
  currentRevision: 'aaaaaaaaaaaaaaaa',
  channel: 'prod',
  currentSourceUrl: 'https://github.com/emojiiii/mediaHub/commit/aaaaaaaaaaaaaaaa',
  latestBuild: {
    version: 'prod-bbbbbbbbbbbb',
    revision: 'bbbbbbbbbbbbbbbb',
    sourceUrl: 'https://github.com/emojiiii/mediaHub/actions/runs/1',
    publishedAt: '2026-08-07T00:00:00.000Z',
  },
  hasUpdate: true,
  updateEnabled: true,
  warning: null,
  operation: {
    phase: 'idle',
    operationId: null,
    fromVersion: null,
    targetVersion: null,
    startedAt: null,
    completedAt: null,
    message: null,
  },
}

describe('console helpers and standalone controls', () => {
  it('在管理员版本面板中确认更新并允许重新检查版本', async () => {
    const user = userEvent.setup()
    const onRefresh = vi.fn()
    const onUpdate = vi.fn()
    vi.spyOn(window, 'confirm').mockReturnValue(true)
    render(<AdminSystemVersionPanel version={SYSTEM_VERSION} loading={false} refreshing={false} error={null} updateError={null} updating={false} onRefresh={onRefresh} onUpdate={onUpdate} />)

    expect(screen.getByText('prod-aaaaaaaaaaaa')).toBeInTheDocument()
    expect(screen.getByText('prod-bbbbbbbbbbbb')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '重新检查系统版本' }))
    expect(onRefresh).toHaveBeenCalledOnce()
    await user.click(screen.getByRole('button', { name: '更新到 prod-bbbbbbbbbbbb' }))
    expect(onUpdate).toHaveBeenCalledOnce()
  })

  it('系统更新运行时保持轮询状态并禁用重复触发', () => {
    render(<AdminSystemVersionPanel version={{ ...SYSTEM_VERSION, operation: { ...SYSTEM_VERSION.operation, phase: 'running', message: '已提交镜像检查' } }} loading={false} refreshing={false} error={null} updateError={null} updating={false} onRefresh={vi.fn()} onUpdate={vi.fn()} />)

    expect(screen.getByText('更新中')).toBeInTheDocument()
    expect(screen.getByText('已提交镜像检查')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '正在更新' })).toBeDisabled()
  })

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

  it('为公开对象复制无 token 原链和短链，并保留私有对象签名链接', async () => {
    const user = userEvent.setup()
    const clipboard = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: clipboard } })
    const publicUrl = 'https://media.example.test/app_test/images/sample.png'
    vi.spyOn(api, 'getPublicUrl').mockResolvedValue({ url: publicUrl, expiresAt: '' })
    vi.spyOn(api, 'createShortLink').mockResolvedValue({ code: 'sample', url: 'https://media.example.test/s/sample', targetUrl: '/app_test/images/sample.png' })
    const queryClient = new QueryClient()
    render(<QueryClientProvider client={queryClient}><ObjectRowActions item={{ ...OBJECT, visibility: '公开' }} deleting={false} onPreview={vi.fn()} onEdit={vi.fn()} onDelete={vi.fn()} /></QueryClientProvider>)

    await user.click(screen.getByRole('button', { name: '复制 sample.png 普通链接' }))
    await waitFor(() => expect(clipboard).toHaveBeenCalledWith(publicUrl))
    await user.click(screen.getByRole('button', { name: '复制 sample.png 短链' }))
    await waitFor(() => expect(clipboard).toHaveBeenCalledWith('https://media.example.test/s/sample'))
  })

  it('私有对象普通链接使用签名 URL，短链按钮保持禁用', async () => {
    const user = userEvent.setup()
    const clipboard = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: clipboard } })
    const signedUrl = 'https://media.example.test/app_test/images/sample.png?token=secret'
    vi.spyOn(api, 'getSignedUrl').mockResolvedValue({ url: signedUrl, expiresAt: '2026-07-19T01:00:00.000Z' })
    const queryClient = new QueryClient()
    render(<QueryClientProvider client={queryClient}><ObjectRowActions item={OBJECT} deleting={false} onPreview={vi.fn()} onEdit={vi.fn()} onDelete={vi.fn()} /></QueryClientProvider>)

    await user.click(screen.getByRole('button', { name: '复制 sample.png 普通链接' }))
    await waitFor(() => expect(clipboard).toHaveBeenCalledWith(signedUrl))
    expect(screen.getByRole('button', { name: '复制 sample.png 短链' })).toBeDisabled()
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
