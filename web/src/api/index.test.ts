import { afterEach, describe, expect, it, vi } from 'vitest'

afterEach(() => {
  vi.unstubAllGlobals()
  vi.resetModules()
})

describe('MediaHub API facade', () => {
  it('updates a Webhook without rotating its secret', async () => {
    const fetchMock = vi.fn(async (_request: Request) => new Response(JSON.stringify({
      endpoint: {
        id: 'webhook-id',
        url: 'https://example.com/hook',
        events: ['media.created'],
        enabled: true,
      },
      secret: null,
    }), { status: 200, headers: { 'Content-Type': 'application/json' } }))
    vi.stubGlobal('fetch', fetchMock)
    const { api } = await import('./index')
    api.setApplication('app_test')

    await api.updateWebhook('webhook-id', {
      url: 'https://example.com/hook',
      events: ['media.created'],
      enabled: true,
    })

    const request = fetchMock.mock.calls[0][0]
    expect(request.method).toBe('PATCH')
    expect(request.headers.get('X-MediaHub-App-Id')).toBe('app_test')
    await expect(request.clone().json()).resolves.toEqual({
      url: 'https://example.com/hook',
      events: ['media.created'],
      enabled: true,
      rotate_secret: false,
    })
    api.setApplication(undefined)
  })
})
