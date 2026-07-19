import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { SPREADSHEET_PREVIEW_MAX_SOURCE_BYTES, type SpreadsheetParseResult } from './spreadsheet-protocol'
import SpreadsheetPreviewPlugin, { SPREADSHEET_PARSE_TIMEOUT_MS } from './SpreadsheetPreviewPlugin'

class MockWorker {
  static instances: MockWorker[] = []
  onerror: ((event: ErrorEvent) => unknown) | null = null
  onmessage: ((event: MessageEvent) => unknown) | null = null
  onmessageerror: ((event: MessageEvent) => unknown) | null = null
  postMessage = vi.fn()
  terminate = vi.fn()

  constructor(public url: URL, public options?: WorkerOptions) {
    MockWorker.instances.push(this)
  }

  emit(data: unknown) {
    this.onmessage?.({ data } as MessageEvent)
  }
}

const originalWorker = globalThis.Worker

beforeEach(() => {
  MockWorker.instances = []
  Object.defineProperty(globalThis, 'Worker', { configurable: true, writable: true, value: MockWorker })
})

afterEach(() => {
  cleanup()
  vi.useRealTimers()
  Object.defineProperty(globalThis, 'Worker', { configurable: true, writable: true, value: originalWorker })
})

describe('SpreadsheetPreviewPlugin', () => {
  it('parses in a module Worker and supports sheet switching and cell search', async () => {
    render(<SpreadsheetPreviewPlugin fileName="report.xlsx" mimeType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" size={4096} url="/signed/report.xlsx" />)
    const worker = MockWorker.instances[0]
    expect(worker.options).toEqual({ type: 'module' })
    expect(worker.postMessage).toHaveBeenCalledWith({ type: 'parse', url: '/signed/report.xlsx', sourceSize: 4096 })

    act(() => worker.emit({ type: 'success', result: workbookResult() }))

    expect(await screen.findByTestId('spreadsheet-preview')).toHaveAttribute('data-viewer-plugin', 'spreadsheet')
    expect(screen.getByRole('table', { name: '订单 数据' })).toBeInTheDocument()
    expect(screen.getByText('北京')).toBeInTheDocument()
    expect(worker.terminate).toHaveBeenCalledOnce()

    fireEvent.change(screen.getByRole('searchbox', { name: '搜索表格' }), { target: { value: 'alpha' } })
    expect(screen.getByText('2 个单元格匹配')).toBeInTheDocument()
    expect(screen.getByText('2 个匹配行')).toBeInTheDocument()
    expect(screen.queryByText('北京')).not.toBeInTheDocument()

    fireEvent.change(screen.getByRole('combobox', { name: '工作表' }), { target: { value: '1' } })
    expect(screen.getByRole('table', { name: '汇总 数据' })).toBeInTheDocument()
    expect(screen.getByText('总计')).toBeInTheDocument()
    expect(screen.getByRole('searchbox', { name: '搜索表格' })).toHaveValue('')
  })

  it('paginates filtered workbook rows without reparsing the file', async () => {
    const rows = Array.from({ length: 60 }, (_, index) => [`row-${index + 1}`, String(index + 1)])
    render(<SpreadsheetPreviewPlugin fileName="rows.csv" mimeType="text/csv" size={1024} url="/signed/rows.csv" />)
    const worker = MockWorker.instances[0]
    act(() => worker.emit({
      type: 'success',
      result: {
        sheets: [{ name: 'Sheet1', rows, columnLabels: ['A', 'B'], startRow: 1, sourceRowCount: 60, sourceColumnCount: 2, truncated: false }],
        sheetCount: 1,
        truncated: false,
        truncationReasons: [],
      },
    }))

    expect(await screen.findByText('row-1')).toBeInTheDocument()
    expect(screen.queryByText('row-51')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '下一页' }))
    expect(screen.getByText('row-51')).toBeInTheDocument()
    expect(screen.queryByText('row-1')).not.toBeInTheDocument()
    expect(screen.getByText('第 2 / 2 页')).toBeInTheDocument()
    expect(MockWorker.instances).toHaveLength(1)
    expect(worker.postMessage).toHaveBeenCalledOnce()
  })

  it('surfaces truncation reasons while keeping extracted rows readable', async () => {
    render(<SpreadsheetPreviewPlugin fileName="large.xlsx" mimeType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" size={1024} url="/signed/large.xlsx" />)
    const worker = MockWorker.instances[0]
    act(() => worker.emit({
      type: 'success',
      result: {
        sheets: [{ name: 'Data', rows: [['保留数据']], columnLabels: ['A'], startRow: 1, sourceRowCount: 30000, sourceColumnCount: 1, truncated: true, truncationReason: '行数超过 20,000 行' }],
        sheetCount: 1,
        truncated: true,
        truncationReasons: ['Data：行数超过 20,000 行'],
      },
    }))

    expect(await screen.findByText('保留数据')).toBeInTheDocument()
    expect(screen.getByText(/数据已截断：Data：行数超过 20,000 行/)).toBeInTheDocument()
    expect(screen.getByText('已加载 1 / 30,000 行')).toBeInTheDocument()
  })

  it('terminates timed out Workers and does not start one above the source limit', () => {
    vi.useFakeTimers()
    const view = render(<SpreadsheetPreviewPlugin fileName="slow.xlsx" mimeType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" size={1024} url="/signed/slow.xlsx" />)
    const worker = MockWorker.instances[0]
    act(() => vi.advanceTimersByTime(SPREADSHEET_PARSE_TIMEOUT_MS))
    expect(screen.getByText('表格解析超时，请重试')).toBeInTheDocument()
    expect(worker.terminate).toHaveBeenCalledOnce()
    view.unmount()

    render(<SpreadsheetPreviewPlugin fileName="huge.xlsx" mimeType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" size={SPREADSHEET_PREVIEW_MAX_SOURCE_BYTES + 1} url="/signed/huge.xlsx" />)
    expect(screen.getByText('表格超过在线预览上限')).toBeInTheDocument()
    expect(screen.getByText(/100 MB/)).toBeInTheDocument()
    expect(MockWorker.instances).toHaveLength(1)
  })

  it('creates a fresh Worker after a parse failure is retried', async () => {
    render(<SpreadsheetPreviewPlugin fileName="broken.xlsx" mimeType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" size={1024} url="/signed/broken.xlsx" />)
    const firstWorker = MockWorker.instances[0]
    act(() => firstWorker.emit({ type: 'error', error: '文件结构损坏' }))
    expect(await screen.findByText('文件结构损坏')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '重试' }))
    await waitFor(() => expect(MockWorker.instances).toHaveLength(2))
    expect(MockWorker.instances[1].postMessage).toHaveBeenCalledWith({ type: 'parse', url: '/signed/broken.xlsx', sourceSize: 1024 })
  })
})

function workbookResult(): SpreadsheetParseResult {
  return {
    sheets: [
      {
        name: '订单',
        rows: [
          ['城市', '标签'],
          ['北京', '普通'],
          ['上海', 'alpha alpha'],
          ['深圳', 'alpha'],
        ],
        columnLabels: ['A', 'B'],
        startRow: 1,
        sourceRowCount: 4,
        sourceColumnCount: 2,
        truncated: false,
      },
      {
        name: '汇总',
        rows: [['总计', '4']],
        columnLabels: ['A', 'B'],
        startRow: 1,
        sourceRowCount: 1,
        sourceColumnCount: 2,
        truncated: false,
      },
    ],
    sheetCount: 2,
    truncated: false,
    truncationReasons: [],
  }
}
