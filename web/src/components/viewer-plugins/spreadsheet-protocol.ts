import { BUFFERED_PREVIEW_MAX_BYTES } from './BufferedPreviewPolicy'

export const SPREADSHEET_PREVIEW_MAX_SOURCE_BYTES = BUFFERED_PREVIEW_MAX_BYTES
export const SPREADSHEET_MAX_SHEETS = 32
export const SPREADSHEET_MAX_ROWS_PER_SHEET = 20_000
export const SPREADSHEET_MAX_COLUMNS = 256
export const SPREADSHEET_MAX_TOTAL_CELLS = 500_000
export const SPREADSHEET_MAX_CELL_CHARACTERS = 4_096

export type SpreadsheetParseRequest = {
  type: 'parse'
  url: string
  sourceSize: number
}

export type SpreadsheetSheet = {
  name: string
  rows: string[][]
  columnLabels: string[]
  startRow: number
  sourceRowCount: number
  sourceColumnCount: number
  truncated: boolean
  truncationReason?: string
}

export type SpreadsheetParseResult = {
  sheets: SpreadsheetSheet[]
  sheetCount: number
  truncated: boolean
  truncationReasons: string[]
}

export type SpreadsheetWorkerSuccess = {
  type: 'success'
  result: SpreadsheetParseResult
}

export type SpreadsheetWorkerFailure = {
  type: 'error'
  error: string
}

export type SpreadsheetWorkerResponse = SpreadsheetWorkerSuccess | SpreadsheetWorkerFailure

export function isSpreadsheetParseRequest(value: unknown): value is SpreadsheetParseRequest {
  if (!value || typeof value !== 'object') return false
  const request = value as Partial<SpreadsheetParseRequest>
  return request.type === 'parse'
    && typeof request.url === 'string'
    && request.url.length > 0
    && typeof request.sourceSize === 'number'
    && Number.isFinite(request.sourceSize)
    && request.sourceSize >= 0
}
