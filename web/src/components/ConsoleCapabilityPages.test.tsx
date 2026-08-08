import { cleanup, render, screen } from '@testing-library/react'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { afterEach, describe, expect, it } from 'vitest'

import { GovernanceFeaturePage, PreviewCenterPage, VariantCenterPage } from './ConsoleCapabilityPages'

afterEach(cleanup)

function renderPage(page: React.ReactNode) {
  return render(<MemoryRouter initialEntries={['/app/app-demo/page']}><Routes><Route path="/app/:appId/page" element={page} /></Routes></MemoryRouter>)
}

describe('console capability pages', () => {
  it('shows real preview capabilities and links back to the object browser', () => {
    renderPage(<PreviewCenterPage />)
    expect(screen.getByRole('heading', { name: '不下载，也能看懂几乎所有文件' })).toBeInTheDocument()
    expect(screen.getByText('表格与数据库')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: /打开文件浏览器/ })).toHaveAttribute('href', '/app/app-demo/objects')
  })

  it('distinguishes available image variants from planned video work', () => {
    renderPage(<VariantCenterPage />)
    expect(screen.getByText('交互式图片 Variant')).toBeInTheDocument()
    expect(screen.getByText('视频 Variant')).toBeInTheDocument()
    expect(screen.getAllByText('S3 重构后接入').length).toBeGreaterThan(0)
  })

  it('does not present versioning as already implemented', () => {
    renderPage(<GovernanceFeaturePage kind="versioning" />)
    expect(screen.getByRole('heading', { name: 'Bucket Versioning' })).toBeInTheDocument()
    expect(screen.getAllByText('S3 重构后接入').length).toBeGreaterThan(0)
  })
})
