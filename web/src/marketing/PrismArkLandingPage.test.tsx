import { cleanup, render, screen, within } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'
import type { ReactElement } from 'react'

import { ThemeProvider } from '../theme'

import { PRISMARK_PAGE_META, PrismArkLandingPage } from './PrismArkLandingPage'

function renderLanding(element: ReactElement = <PrismArkLandingPage />) {
  return render(<ThemeProvider>{element}</ThemeProvider>)
}

afterEach(cleanup)

describe('PrismArkLandingPage', () => {
  it('renders a semantic, crawlable product page with exactly one H1', () => {
    const { container } = renderLanding(<PrismArkLandingPage />)

    expect(screen.getByRole('banner')).toBeInTheDocument()
    expect(screen.getByRole('main')).toHaveAttribute('id', 'main-content')
    expect(screen.getByRole('contentinfo')).toBeInTheDocument()
    expect(screen.getAllByRole('heading', { level: 1 })).toHaveLength(1)
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('存下每一个对象')
    expect(container.querySelectorAll('section[id]').length).toBeGreaterThanOrEqual(6)
  })

  it('exposes working anchor navigation for the core product sections', () => {
    renderLanding(<PrismArkLandingPage />)

    const navigation = screen.getByRole('navigation', { name: '主导航' })
    expect(within(navigation).getByRole('link', { name: '产品能力' })).toHaveAttribute('href', '#capabilities')
    expect(within(navigation).getByRole('link', { name: '文件体验' })).toHaveAttribute('href', '#file-experience')
    expect(within(navigation).getByRole('link', { name: '协议与架构' })).toHaveAttribute('href', '#architecture')
    expect(within(navigation).getByRole('link', { name: '产品路线' })).toHaveAttribute('href', '#roadmap')
  })

  it('describes the real product differentiation without fabricated traction claims', () => {
    const { container } = renderLanding(<PrismArkLandingPage />)

    expect(screen.getByText('对象不应该只是一个下载链接')).toBeInTheDocument()
    expect(screen.getByText('像文件管理器一样自然，像对象存储一样可靠')).toBeInTheDocument()
    expect(screen.getByText('原图只存一份，交付可以千变万化')).toBeInTheDocument()
    expect(screen.getByText('S3 是主协议，WebDAV 是兼容层')).toBeInTheDocument()
    expect(screen.getByText('AI 内容理解')).toBeInTheDocument()
    expect(screen.getByText('路线方向')).toBeInTheDocument()

    const text = container.textContent ?? ''
    expect(text).not.toMatch(/客户评价|领先客户|用户数量|99\.9+%|全球第一/)
  })

  it('supports integration-provided console, docs and source destinations', () => {
    renderLanding(
      <PrismArkLandingPage
        consoleHref="/console"
        docsHref="/docs/deploy"
        sourceHref="https://example.test/source"
      />,
    )

    expect(screen.getAllByRole('link', { name: /打开控制台/ })[0]).toHaveAttribute('href', '/console')
    expect(screen.getAllByRole('link', { name: /查看部署方式|部署说明/ })[0]).toHaveAttribute('href', '/docs/deploy')
    expect(screen.getByRole('link', { name: '获取部署说明' })).toHaveAttribute('href', 'https://example.test/source')
  })

  it('includes truthful SoftwareApplication structured data', () => {
    const { container } = renderLanding(<PrismArkLandingPage />)
    const structuredData = container.querySelector('script[type="application/ld+json"]')
    const data = JSON.parse(structuredData?.textContent ?? '{}') as Record<string, unknown>

    expect(PRISMARK_PAGE_META.title).toContain('PrismArk')
    expect(data['@type']).toBe('SoftwareApplication')
    expect(data.name).toBe('PrismArk')
    expect(data).not.toHaveProperty('aggregateRating')
    expect(data).not.toHaveProperty('offers')
  })
})
