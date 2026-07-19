import { afterEach, describe, expect, it, vi } from 'vitest'

import { deferViewerRootCleanup } from './ViewerShared'

afterEach(() => document.body.replaceChildren())

describe('deferViewerRootCleanup', () => {
  it('waits until the active parent render has completed before unmounting a nested React root', async () => {
    const mount = document.createElement('div')
    const unmount = vi.fn()
    const cleanup = vi.fn()
    document.body.append(mount)

    deferViewerRootCleanup({ unmount }, mount, cleanup)

    expect(unmount).not.toHaveBeenCalled()
    expect(cleanup).not.toHaveBeenCalled()
    expect(mount.isConnected).toBe(true)

    await Promise.resolve()

    expect(unmount).toHaveBeenCalledOnce()
    expect(cleanup).toHaveBeenCalledOnce()
    expect(mount.isConnected).toBe(false)
  })
})
