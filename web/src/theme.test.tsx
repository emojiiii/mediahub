import { act, cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { ThemeToggle } from './components/ThemeToggle'
import { THEME_STORAGE_KEY, ThemeProvider, useTheme } from './theme'

const DARK_MEDIA_QUERY = '(prefers-color-scheme: dark)'

let systemDark = false
let mediaListeners: Set<(event: MediaQueryListEvent) => void>

function installMatchMedia() {
  mediaListeners = new Set()
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    writable: true,
    value: vi.fn((query: string) => ({
      matches: systemDark,
      media: query,
      onchange: null,
      addEventListener: (_type: string, listener: (event: MediaQueryListEvent) => void) => mediaListeners.add(listener),
      removeEventListener: (_type: string, listener: (event: MediaQueryListEvent) => void) => mediaListeners.delete(listener),
      addListener: (listener: (event: MediaQueryListEvent) => void) => mediaListeners.add(listener),
      removeListener: (listener: (event: MediaQueryListEvent) => void) => mediaListeners.delete(listener),
      dispatchEvent: vi.fn(),
    })),
  })
}

function changeSystemTheme(dark: boolean) {
  systemDark = dark
  const event = { matches: dark, media: DARK_MEDIA_QUERY } as MediaQueryListEvent
  act(() => mediaListeners.forEach((listener) => listener(event)))
}

function ThemeState() {
  const { resolvedTheme, setTheme, theme } = useTheme()
  return (
    <div>
      <output aria-label="主题偏好">{theme}</output>
      <output aria-label="生效主题">{resolvedTheme}</output>
      <button type="button" onClick={() => setTheme('dark')}>切换深色</button>
    </div>
  )
}

beforeEach(() => {
  systemDark = false
  installMatchMedia()
  window.localStorage.clear()
  delete document.documentElement.dataset.theme
  document.documentElement.style.removeProperty('color-scheme')
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
  window.localStorage.clear()
  delete document.documentElement.dataset.theme
  document.documentElement.style.removeProperty('color-scheme')
})

describe('ThemeProvider', () => {
  it('恢复持久化偏好并同步到根节点', () => {
    window.localStorage.setItem(THEME_STORAGE_KEY, 'dark')

    render(<ThemeProvider><ThemeState /></ThemeProvider>)

    expect(screen.getByLabelText('主题偏好')).toHaveTextContent('dark')
    expect(screen.getByLabelText('生效主题')).toHaveTextContent('dark')
    expect(document.documentElement).toHaveAttribute('data-theme', 'dark')
    expect(document.documentElement.style.colorScheme).toBe('dark')
  })

  it('持久化显式选择并立即应用', async () => {
    const user = userEvent.setup()
    render(<ThemeProvider defaultTheme="light"><ThemeState /></ThemeProvider>)

    await user.click(screen.getByRole('button', { name: '切换深色' }))

    expect(document.documentElement).toHaveAttribute('data-theme', 'dark')
    expect(document.documentElement.style.colorScheme).toBe('dark')
    await waitFor(() => expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe('dark'))
  })

  it('在 system 偏好下响应操作系统主题变化', () => {
    render(<ThemeProvider><ThemeState /></ThemeProvider>)

    expect(screen.getByLabelText('主题偏好')).toHaveTextContent('system')
    expect(document.documentElement).toHaveAttribute('data-theme', 'light')
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe('system')

    changeSystemTheme(true)

    expect(screen.getByLabelText('主题偏好')).toHaveTextContent('system')
    expect(screen.getByLabelText('生效主题')).toHaveTextContent('dark')
    expect(document.documentElement).toHaveAttribute('data-theme', 'dark')
    expect(document.documentElement.style.colorScheme).toBe('dark')
  })

  it('忽略无效存储值并使用默认偏好', () => {
    window.localStorage.setItem(THEME_STORAGE_KEY, 'sepia')
    render(<ThemeProvider defaultTheme="light"><ThemeState /></ThemeProvider>)

    expect(screen.getByLabelText('主题偏好')).toHaveTextContent('light')
    expect(document.documentElement).toHaveAttribute('data-theme', 'light')
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe('light')
  })
})

describe('ThemeToggle', () => {
  it('提供命名分组、三个原生单选项和可见选中状态', async () => {
    const user = userEvent.setup()
    render(<ThemeProvider defaultTheme="light"><ThemeToggle /></ThemeProvider>)

    expect(screen.getByRole('group', { name: '界面主题' })).toBeInTheDocument()
    expect(screen.getByRole('radio', { name: '浅色' })).toBeChecked()
    expect(screen.getByRole('radio', { name: '深色' })).not.toBeChecked()
    expect(screen.getByRole('radio', { name: '跟随系统' })).not.toBeChecked()

    await user.click(screen.getByRole('radio', { name: '深色' }))

    expect(screen.getByRole('radio', { name: '深色' })).toBeChecked()
    expect(document.documentElement).toHaveAttribute('data-theme', 'dark')
    await waitFor(() => expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe('dark'))
  })

  it('紧凑模式仍为每个图标保留无障碍名称', () => {
    render(<ThemeProvider><ThemeToggle label="颜色模式" showLabels={false} /></ThemeProvider>)

    expect(screen.getByRole('group', { name: '颜色模式' })).toBeInTheDocument()
    expect(screen.getByRole('radio', { name: '浅色' })).toBeInTheDocument()
    expect(screen.getByRole('radio', { name: '深色' })).toBeInTheDocument()
    expect(screen.getByRole('radio', { name: '跟随系统' })).toBeInTheDocument()
  })
})
