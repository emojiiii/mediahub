import * as XLSX from 'xlsx'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { SPREADSHEET_PREVIEW_MAX_SOURCE_BYTES, type SpreadsheetWorkerResponse } from './spreadsheet-protocol'

const originalFetch = globalThis.fetch
const originalOnMessage = self.onmessage
const originalPostMessage = self.postMessage

beforeEach(() => {
  vi.resetModules()
  self.onmessage = null
})

afterEach(() => {
  vi.restoreAllMocks()
  Object.defineProperty(globalThis, 'fetch', { configurable: true, writable: true, value: originalFetch })
  Object.defineProperty(self, 'postMessage', { configurable: true, writable: true, value: originalPostMessage })
  self.onmessage = originalOnMessage
})

describe('spreadsheet Worker', () => {
  it('parses multiple worksheets into bounded display-only strings', async () => {
    const workbook = XLSX.utils.book_new()
    XLSX.utils.book_append_sheet(workbook, XLSX.utils.aoa_to_sheet([
      ['名称', '数量'],
      ['Alpha', 12],
    ]), '订单')
    XLSX.utils.book_append_sheet(workbook, XLSX.utils.aoa_to_sheet([
      ['状态'],
      ['完成'],
    ]), '汇总')
    const bytes = XLSX.write(workbook, { type: 'array', bookType: 'xlsx' }) as ArrayBuffer
    const postMessage = vi.fn<(message: SpreadsheetWorkerResponse) => void>()
    Object.defineProperty(self, 'postMessage', { configurable: true, writable: true, value: postMessage })
    Object.defineProperty(globalThis, 'fetch', {
      configurable: true,
      writable: true,
      value: vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        headers: { get: () => String(bytes.byteLength) },
        body: null,
        arrayBuffer: async () => bytes,
      }),
    })

    await import('./spreadsheet.worker')
    self.onmessage?.({ data: { type: 'parse', url: '/signed/report.xlsx', sourceSize: bytes.byteLength } } as MessageEvent)
    await vi.waitFor(() => expect(postMessage).toHaveBeenCalledOnce())

    const response = postMessage.mock.calls[0][0]
    expect(response.type).toBe('success')
    if (response.type !== 'success') return
    expect(response.result.sheetCount).toBe(2)
    expect(response.result.sheets.map((sheet) => sheet.name)).toEqual(['订单', '汇总'])
    expect(response.result.sheets[0]).toMatchObject({
      rows: [['名称', '数量'], ['Alpha', '12']],
      columnLabels: ['A', 'B'],
      sourceRowCount: 2,
      sourceColumnCount: 2,
      truncated: false,
    })
  })

  it('rejects oversized sources before fetching them', async () => {
    const postMessage = vi.fn<(message: SpreadsheetWorkerResponse) => void>()
    const fetch = vi.fn()
    Object.defineProperty(self, 'postMessage', { configurable: true, writable: true, value: postMessage })
    Object.defineProperty(globalThis, 'fetch', { configurable: true, writable: true, value: fetch })

    await import('./spreadsheet.worker')
    self.onmessage?.({
      data: { type: 'parse', url: '/signed/huge.xlsx', sourceSize: SPREADSHEET_PREVIEW_MAX_SOURCE_BYTES + 1 },
    } as MessageEvent)
    await vi.waitFor(() => expect(postMessage).toHaveBeenCalledOnce())

    expect(postMessage.mock.calls[0][0]).toEqual({ type: 'error', error: expect.stringContaining('100 MB') })
    expect(fetch).not.toHaveBeenCalled()
  })
})
