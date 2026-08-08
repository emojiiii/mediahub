import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

const viewerSpy = vi.fn()

vi.mock('./ObjectFileViewer', () => ({
  default: (props: Record<string, unknown>) => {
    viewerSpy(props)
    return <div data-testid="object-file-viewer" />
  },
}))

import { PreviewSurface } from './PreviewSurface'

afterEach(() => {
  cleanup()
  viewerSpy.mockClear()
  document.body.style.removeProperty('overflow')
})

describe('PreviewSurface', () => {
  it('shows file context and safely links to the original object', () => {
    render(<PreviewSurface fileName="report.pdf" mimeType="application/pdf" size={1536} url="/signed/report.pdf" theme="dark" />)

    expect(screen.getByText('report.pdf')).toBeInTheDocument()
    expect(screen.getByText('application/pdf · 1.5 KB')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: '下载原文件' })).toHaveAttribute('href', '/signed/report.pdf')
    expect(screen.getByRole('link', { name: '下载原文件' })).toHaveAttribute('rel', expect.stringContaining('noopener'))
    expect(viewerSpy).toHaveBeenLastCalledWith(expect.objectContaining({ theme: 'dark' }))
  })

  it('reloads the viewer and supports fullscreen with Escape', () => {
    render(<PreviewSurface fileName="demo.txt" mimeType="text/plain" size={12} url="/demo.txt" theme="light" />)
    const initialCalls = viewerSpy.mock.calls.length

    fireEvent.click(screen.getByRole('button', { name: '重新加载预览' }))
    expect(viewerSpy.mock.calls.length).toBeGreaterThan(initialCalls)

    fireEvent.click(screen.getByRole('button', { name: '全屏预览' }))
    expect(screen.getByTestId('preview-surface')).toHaveClass('fixed')
    expect(document.body.style.overflow).toBe('hidden')

    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.getByTestId('preview-surface')).not.toHaveClass('fixed')
    expect(document.body.style.overflow).toBe('')
  })
})
