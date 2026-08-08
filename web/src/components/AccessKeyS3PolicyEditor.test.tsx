import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { api, type AccessKey } from '../api'
import { AccessKeyS3PolicyEditor } from './AccessKeyS3PolicyEditor'

const ACCESS_KEY: AccessKey = {
  id: 'mh_ak_policy',
  name: 'preview-reader',
  permissions: ['media:read'],
  scope: 'media:read',
  lastUsed: '暂无使用记录',
  expires: '不过期',
  expiresAt: null,
  status: '有效',
}

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

function renderEditor() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  render(<QueryClientProvider client={queryClient}><AccessKeyS3PolicyEditor accessKey={ACCESS_KEY} onClose={vi.fn()} /></QueryClientProvider>)
}

describe('AccessKeyS3PolicyEditor', () => {
  it('keeps missing policy deny-by-default until the user explicitly fills and saves a template', async () => {
    const user = userEvent.setup()
    vi.spyOn(api, 'getAccessKeyS3Policy').mockResolvedValue(null)
    const put = vi.spyOn(api, 'putAccessKeyS3Policy').mockImplementation(async (_id, policy) => policy)
    renderEditor()

    expect(await screen.findByText('未配置')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '保存 Policy' })).toBeDisabled()
    expect(put).not.toHaveBeenCalled()

    await user.click(screen.getByRole('button', { name: '填入最小安全模板' }))
    expect((screen.getByRole('textbox', { name: 'S3 Identity Policy JSON' }) as HTMLTextAreaElement).value).toContain('"Sid": "DenyAll"')
    expect(put).not.toHaveBeenCalled()

    await user.click(screen.getByRole('button', { name: '保存 Policy' }))
    await waitFor(() => expect(put).toHaveBeenCalledWith('mh_ak_policy', expect.objectContaining({
      Statement: [expect.objectContaining({ Effect: 'Deny', Action: 's3:*', Resource: '*' })],
    })))
  })

  it('accepts a single Statement object and requires confirmation before deletion', async () => {
    const user = userEvent.setup()
    vi.spyOn(api, 'getAccessKeyS3Policy').mockResolvedValue({
      Version: '2012-10-17',
      Statement: { Effect: 'Deny', Action: 's3:*', Resource: '*' },
    })
    const remove = vi.spyOn(api, 'deleteAccessKeyS3Policy').mockResolvedValue(undefined)
    vi.spyOn(window, 'confirm').mockReturnValue(true)
    renderEditor()

    expect(await screen.findByText('已配置')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '删除 Policy' }))
    await waitFor(() => expect(remove).toHaveBeenCalledWith('mh_ak_policy'))
    expect(await screen.findByRole('status')).toHaveTextContent('默认拒绝')
  })
})
