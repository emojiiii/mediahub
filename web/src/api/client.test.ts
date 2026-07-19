import { afterEach, describe, expect, it, vi } from 'vitest'

import { createMediaHubClient, resolveApiBaseUrl } from './client'

afterEach(() => vi.unstubAllGlobals())

describe('MediaHub generated client', () => {
  it('keeps loopback API requests on the current browser hostname', () => {
    expect(resolveApiBaseUrl('http://localhost:3000/', { hostname: '127.0.0.1' })).toBe('http://127.0.0.1:3000')
    expect(resolveApiBaseUrl('http://127.0.0.1:3000', { hostname: 'localhost' })).toBe('http://localhost:3000')
  })

  it('preserves configured non-loopback API hosts', () => {
    expect(resolveApiBaseUrl('https://api.mediahub.example/v1/', { hostname: 'console.mediahub.example' })).toBe('https://api.mediahub.example/v1')
  })

  it('adds CSRF and selected Application headers to mutations', async () => {
    const fetchMock = vi.fn(async (_request: Request) => new Response(JSON.stringify({ id: 'bucket-id', name: 'images', visibility: 'private', allowed_mime_types: [], lifecycle_rules: [] }), { status: 201, headers: { 'Content-Type': 'application/json' } }))
    vi.stubGlobal('fetch', fetchMock)
    const client = createMediaHubClient('https://mediahub.example', () => 'csrf-token', () => 'app_test')
    const result = await client.POST('/api/v1/buckets', { body: { name: 'images' } })
    expect(result.error).toBeUndefined()
    const request = fetchMock.mock.calls[0][0]
    expect(request.headers.get('X-CSRF-Token')).toBe('csrf-token')
    expect(request.headers.get('X-MediaHub-App-Id')).toBe('app_test')
  })

  it('does not add CSRF to reads', async () => {
    const fetchMock = vi.fn(async (_request: Request) => new Response(JSON.stringify([]), { status: 200, headers: { 'Content-Type': 'application/json' } }))
    vi.stubGlobal('fetch', fetchMock)
    const client = createMediaHubClient('https://mediahub.example', () => 'csrf-token', () => 'app_test')
    await client.GET('/api/v1/buckets')
    const request = fetchMock.mock.calls[0][0]
    expect(request.headers.has('X-CSRF-Token')).toBe(false)
    expect(request.headers.get('X-MediaHub-App-Id')).toBe('app_test')
  })
})
