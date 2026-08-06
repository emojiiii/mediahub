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

  it('builds token-free public object URLs and creates short links for them', async () => {
    const fetchMock = vi.fn(async (request: Request) => {
      const url = new URL(request.url)
      if (request.method === 'GET' && url.pathname === '/api/v1/buckets') {
        return new Response(JSON.stringify([{
          id: 'bucket-id',
          name: 'public assets',
          visibility: 'public',
        }]), { status: 200, headers: { 'Content-Type': 'application/json' } })
      }
      if (request.method === 'GET' && url.pathname === '/api/v1/media') {
        return new Response(JSON.stringify({
          items: [{
            id: 'media-id',
            bucket_id: 'bucket-id',
            object_key: 'campaigns/launch video.mp4',
            visibility: null,
          }],
          common_prefixes: [],
          next_cursor: null,
        }), { status: 200, headers: { 'Content-Type': 'application/json' } })
      }
      if (request.method === 'POST' && url.pathname === '/api/v1/short-links') {
        return new Response(JSON.stringify({
          code: 'launch',
          url: '/s/launch',
          target_url: '/app_test/public%20assets/campaigns/launch%20video.mp4',
          created_at: '2026-08-07T00:00:00Z',
        }), { status: 201, headers: { 'Content-Type': 'application/json' } })
      }
      return new Response(null, { status: 404 })
    })
    vi.stubGlobal('fetch', fetchMock)
    const { api } = await import('./index')
    api.setApplication('app_test')

    const publicLink = await api.getPublicUrl('media-id')
    const publicUrl = new URL(publicLink.url)
    expect(publicUrl.pathname).toBe('/app_test/public%20assets/campaigns/launch%20video.mp4')
    expect(publicUrl.search).toBe('')
    expect(publicLink.expiresAt).toBe('')

    const shortLink = await api.createShortLink(publicLink.url)
    expect(shortLink).toEqual({ code: 'launch', url: 'http://localhost:3000/s/launch', targetUrl: publicUrl.pathname })
    const shortLinkRequest = fetchMock.mock.calls.map(([request]) => request).find((request) => new URL(request.url).pathname === '/api/v1/short-links')
    await expect(shortLinkRequest?.clone().json()).resolves.toEqual({ target_url: publicLink.url })
    api.setApplication(undefined)
  })
})
