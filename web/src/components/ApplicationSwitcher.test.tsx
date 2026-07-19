import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { ApplicationSwitcher, shortAppId } from './ApplicationSwitcher'

afterEach(cleanup)

const applications = [
  { appId: 'app_019f6607ca6f7471a34fa4f7aa0b22b2', name: '默认应用' },
  { appId: 'app_archive', name: '归档素材库' },
]

describe('ApplicationSwitcher', () => {
  it('shows the current application without exposing the full long AppId', () => {
    render(
      <ApplicationSwitcher
        applications={applications}
        currentAppId={applications[0].appId}
        onCreate={vi.fn()}
        onSelect={vi.fn()}
      />,
    )

    expect(screen.getByRole('button', { name: '切换应用，当前为 默认应用' })).toBeInTheDocument()
    expect(screen.getByText(shortAppId(applications[0].appId))).toBeInTheDocument()
    expect(screen.queryByText(applications[0].appId)).not.toBeInTheDocument()
  })

  it('selects applications and exposes the create command through the keyboard menu', async () => {
    const user = userEvent.setup()
    const onSelect = vi.fn()
    const onCreate = vi.fn()
    render(
      <ApplicationSwitcher
        applications={applications}
        currentAppId={applications[0].appId}
        onCreate={onCreate}
        onSelect={onSelect}
      />,
    )

    const trigger = screen.getByRole('button', { name: '切换应用，当前为 默认应用' })
    trigger.focus()
    await user.keyboard('{Enter}')
    await user.click(await screen.findByRole('menuitemradio', { name: /归档素材库/ }))
    expect(onSelect).toHaveBeenCalledWith('app_archive')

    await user.click(trigger)
    await user.click(await screen.findByRole('menuitemradio', { name: '新建应用' }))
    expect(onCreate).toHaveBeenCalledOnce()
  })

  it('disables the trigger while loading', () => {
    render(
      <ApplicationSwitcher
        applications={[]}
        currentAppId=""
        isLoading
        onCreate={vi.fn()}
        onSelect={vi.fn()}
      />,
    )

    expect(screen.getByRole('button', { name: '切换应用，当前为 暂无应用' })).toBeDisabled()
    expect(screen.getByLabelText('正在加载应用')).toBeInTheDocument()
  })
})
