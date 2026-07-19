import { describe, expect, it } from 'vitest'

import {
  isSpreadsheetParseRequest,
  SPREADSHEET_MAX_COLUMNS,
  SPREADSHEET_MAX_ROWS_PER_SHEET,
  SPREADSHEET_MAX_SHEETS,
  SPREADSHEET_MAX_TOTAL_CELLS,
  SPREADSHEET_PREVIEW_MAX_SOURCE_BYTES,
} from './spreadsheet-protocol'

describe('spreadsheet protocol', () => {
  it('accepts only finite, non-negative parse requests with a URL', () => {
    expect(isSpreadsheetParseRequest({ type: 'parse', url: '/signed/data.xlsx', sourceSize: 1024 })).toBe(true)
    expect(isSpreadsheetParseRequest({ type: 'parse', url: '', sourceSize: 1024 })).toBe(false)
    expect(isSpreadsheetParseRequest({ type: 'scan', url: '/signed/data.xlsx', sourceSize: 1024 })).toBe(false)
    expect(isSpreadsheetParseRequest({ type: 'parse', url: '/signed/data.xlsx', sourceSize: -1 })).toBe(false)
    expect(isSpreadsheetParseRequest({ type: 'parse', url: '/signed/data.xlsx', sourceSize: Number.NaN })).toBe(false)
  })

  it('keeps parser resource limits explicit and bounded', () => {
    expect(SPREADSHEET_PREVIEW_MAX_SOURCE_BYTES).toBe(100 * 1024 * 1024)
    expect(SPREADSHEET_MAX_SHEETS).toBe(32)
    expect(SPREADSHEET_MAX_ROWS_PER_SHEET).toBe(20_000)
    expect(SPREADSHEET_MAX_COLUMNS).toBe(256)
    expect(SPREADSHEET_MAX_TOTAL_CELLS).toBe(500_000)
  })
})
