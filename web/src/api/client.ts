import createClient, { type Middleware } from 'openapi-fetch'

import type { paths } from './generated'

type ApiPageLocation = { hostname: string; origin: string }

function isLoopbackHostname(hostname: string): boolean {
  return hostname === 'localhost' || hostname === '127.0.0.1' || hostname === '[::1]' || hostname === '::1'
}

export function resolveApiBaseUrl(
  configuredBaseUrl: string | undefined,
  pageLocation: ApiPageLocation | undefined = typeof window === 'undefined' ? undefined : window.location,
  developmentMode = import.meta.env.DEV,
): string {
  const configured = configuredBaseUrl?.trim()
  const defaultBaseUrl = developmentMode
    ? `http://${pageLocation?.hostname ?? 'localhost'}:3000`
    : pageLocation?.origin ?? 'http://localhost:3000'
  const url = new URL(configured || defaultBaseUrl)
  if (pageLocation && isLoopbackHostname(url.hostname) && isLoopbackHostname(pageLocation.hostname)) url.hostname = pageLocation.hostname
  return url.toString().replace(/\/+$/, '')
}

export function createMediaHubClient(baseUrl = '', readCsrfToken: () => string | undefined = () => undefined, readApplicationId: () => string | undefined = () => undefined) {
  const client = createClient<paths>({
    baseUrl: baseUrl.replace(/\/$/, ''),
    credentials: 'include',
  })
  const headers: Middleware = {
    onRequest({ request }) {
      request.headers.set('Accept', 'application/json')
      if (!['GET', 'HEAD', 'OPTIONS'].includes(request.method)) request.headers.set('X-CSRF-Token', readCsrfToken() ?? '')
      const applicationId = readApplicationId()
      if (applicationId) request.headers.set('X-MediaHub-App-Id', applicationId)
      return request
    },
  }
  client.use(headers)
  return client
}

export type MediaHubClient = ReturnType<typeof createMediaHubClient>
