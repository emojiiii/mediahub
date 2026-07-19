import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { SelectControl } from './SelectControl'

afterEach(cleanup)

const options = [
  { value: '', label: 'All buckets' },
  { value: 'public', label: 'Public', description: 'Visible to everyone' },
  { value: 'private', label: 'Private', description: 'Restricted access' },
] as const

describe('SelectControl', () => {
  it('shows the current value and exposes an accessible trigger and menu', async () => {
    const user = userEvent.setup()
    render(
      <SelectControl
        aria-label="Bucket filter"
        onChange={vi.fn()}
        options={options}
        value="private"
      />,
    )

    const trigger = screen.getByRole('button', { name: 'Bucket filter' })
    expect(trigger).toHaveTextContent('Private')
    expect(trigger).not.toBeDisabled()

    await user.click(trigger)

    expect(await screen.findByRole('menu', { name: 'Bucket filter' })).toBeInTheDocument()
    expect(screen.getByRole('menuitemradio', { name: /Private/ })).toHaveAttribute('aria-checked', 'true')
  })

  it('opens the menu and reports the selected option', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(
      <SelectControl
        aria-label="Visibility"
        onChange={onChange}
        options={options}
        value="private"
      />,
    )

    await user.click(screen.getByRole('button', { name: 'Visibility' }))
    await user.click(await screen.findByRole('menuitemradio', { name: /Public/ }))

    expect(onChange).toHaveBeenCalledOnce()
    expect(onChange).toHaveBeenCalledWith('public')
  })

  it('maps the internal empty option key back to an empty string', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(
      <SelectControl
        aria-label="Bucket filter"
        onChange={onChange}
        options={options}
        value="public"
      />,
    )

    await user.click(screen.getByRole('button', { name: 'Bucket filter' }))
    await user.click(await screen.findByRole('menuitemradio', { name: 'All buckets' }))

    expect(onChange).toHaveBeenCalledWith('')
  })

  it('disables the trigger and does not open the menu', async () => {
    const user = userEvent.setup()
    render(
      <SelectControl
        aria-label="Disabled filter"
        isDisabled
        onChange={vi.fn()}
        options={options}
        value="private"
      />,
    )

    const trigger = screen.getByRole('button', { name: 'Disabled filter' })
    expect(trigger).toBeDisabled()

    await user.click(trigger)

    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
  })
})
