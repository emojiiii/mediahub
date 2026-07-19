import * as XLSX from 'xlsx'

import {
  isSpreadsheetParseRequest,
  SPREADSHEET_MAX_CELL_CHARACTERS,
  SPREADSHEET_MAX_COLUMNS,
  SPREADSHEET_MAX_ROWS_PER_SHEET,
  SPREADSHEET_MAX_SHEETS,
  SPREADSHEET_MAX_TOTAL_CELLS,
  SPREADSHEET_PREVIEW_MAX_SOURCE_BYTES,
  type SpreadsheetParseRequest,
  type SpreadsheetParseResult,
  type SpreadsheetSheet,
  type SpreadsheetWorkerResponse,
} from './spreadsheet-protocol'

type WorkerScope = {
  onmessage: ((event: MessageEvent<unknown>) => void) | null
  postMessage(message: SpreadsheetWorkerResponse): void
}

const workerScope = self as unknown as WorkerScope
let parseQueue = Promise.resolve()

workerScope.onmessage = (event) => {
  parseQueue = parseQueue.then(() => handleMessage(event.data))
}

async function handleMessage(message: unknown): Promise<void> {
  if (!isSpreadsheetParseRequest(message)) {
    postFailure('表格预览请求无效。')
    return
  }

  try {
    const result = await parseSpreadsheet(message)
    workerScope.postMessage({ type: 'success', result })
  } catch (cause) {
    postFailure(spreadsheetErrorMessage(cause))
  }
}

async function parseSpreadsheet(request: SpreadsheetParseRequest): Promise<SpreadsheetParseResult> {
  assertSourceSize(request.sourceSize)
  const buffer = await fetchSpreadsheetBuffer(request.url)
  const workbook = XLSX.read(buffer, {
    type: 'array',
    sheetRows: SPREADSHEET_MAX_ROWS_PER_SHEET,
    cellDates: true,
    cellFormula: false,
    cellHTML: false,
    cellStyles: false,
    bookVBA: false,
  })

  const sheetNames = workbook.SheetNames.slice(0, SPREADSHEET_MAX_SHEETS)
  const truncationReasons: string[] = []
  if (workbook.SheetNames.length > sheetNames.length) {
    truncationReasons.push(`工作表数量超过 ${SPREADSHEET_MAX_SHEETS} 个，仅加载前 ${SPREADSHEET_MAX_SHEETS} 个。`)
  }

  const sheets: SpreadsheetSheet[] = []
  let remainingCells = SPREADSHEET_MAX_TOTAL_CELLS
  for (const name of sheetNames) {
    const sheet = extractSheet(name, workbook.Sheets[name], remainingCells)
    sheets.push(sheet)
    remainingCells -= sheet.rows.length * sheet.columnLabels.length
    if (sheet.truncationReason) truncationReasons.push(`${name}：${sheet.truncationReason}`)
  }

  return {
    sheets,
    sheetCount: workbook.SheetNames.length,
    truncated: truncationReasons.length > 0,
    truncationReasons,
  }
}

function extractSheet(name: string, worksheet: XLSX.WorkSheet | undefined, remainingCells: number): SpreadsheetSheet {
  const reference = worksheet?.['!fullref'] || worksheet?.['!ref']
  if (!worksheet || !reference) {
    return {
      name,
      rows: [],
      columnLabels: [],
      startRow: 1,
      sourceRowCount: 0,
      sourceColumnCount: 0,
      truncated: false,
    }
  }

  const sourceRange = XLSX.utils.decode_range(reference)
  const sourceRowCount = Math.max(0, sourceRange.e.r - sourceRange.s.r + 1)
  const sourceColumnCount = Math.max(0, sourceRange.e.c - sourceRange.s.c + 1)
  const visibleColumnCount = Math.min(sourceColumnCount, SPREADSHEET_MAX_COLUMNS, remainingCells)
  const cellLimitedRows = visibleColumnCount > 0 ? Math.floor(remainingCells / visibleColumnCount) : 0
  const visibleRowCount = Math.min(sourceRowCount, SPREADSHEET_MAX_ROWS_PER_SHEET, cellLimitedRows)
  const truncationParts: string[] = []

  if (sourceColumnCount > visibleColumnCount) {
    const reason = visibleColumnCount < Math.min(sourceColumnCount, SPREADSHEET_MAX_COLUMNS)
      ? `工作簿总单元格数超过 ${SPREADSHEET_MAX_TOTAL_CELLS.toLocaleString('en-US')} 个`
      : `列数超过 ${SPREADSHEET_MAX_COLUMNS} 列`
    truncationParts.push(reason)
  }
  if (sourceRowCount > visibleRowCount) {
    const reason = visibleRowCount < Math.min(sourceRowCount, SPREADSHEET_MAX_ROWS_PER_SHEET)
      ? `工作簿总单元格数超过 ${SPREADSHEET_MAX_TOTAL_CELLS.toLocaleString('en-US')} 个`
      : `行数超过 ${SPREADSHEET_MAX_ROWS_PER_SHEET.toLocaleString('en-US')} 行`
    truncationParts.push(reason)
  }

  if (visibleColumnCount === 0 || visibleRowCount === 0) {
    return {
      name,
      rows: [],
      columnLabels: [],
      startRow: sourceRange.s.r + 1,
      sourceRowCount,
      sourceColumnCount,
      truncated: sourceRowCount > 0 || sourceColumnCount > 0,
      truncationReason: uniqueReasons(truncationParts).join('；') || '已达到工作簿总单元格上限',
    }
  }

  const visibleRange = {
    s: sourceRange.s,
    e: {
      r: sourceRange.s.r + visibleRowCount - 1,
      c: sourceRange.s.c + visibleColumnCount - 1,
    },
  }
  const rawRows = XLSX.utils.sheet_to_json<unknown[]>(worksheet, {
    header: 1,
    raw: false,
    defval: '',
    blankrows: true,
    range: visibleRange,
  })
  const rows = Array.from({ length: visibleRowCount }, (_, rowIndex) => {
    const row = rawRows[rowIndex] ?? []
    return Array.from({ length: visibleColumnCount }, (_, columnIndex) => normalizeCell(row[columnIndex]))
  })

  return {
    name,
    rows,
    columnLabels: Array.from(
      { length: visibleColumnCount },
      (_, index) => XLSX.utils.encode_col(sourceRange.s.c + index),
    ),
    startRow: sourceRange.s.r + 1,
    sourceRowCount,
    sourceColumnCount,
    truncated: truncationParts.length > 0,
    ...(truncationParts.length > 0 ? { truncationReason: uniqueReasons(truncationParts).join('；') } : {}),
  }
}

function normalizeCell(value: unknown): string {
  let text: string
  if (value === null || value === undefined) text = ''
  else if (value instanceof Date) text = value.toISOString()
  else if (typeof value === 'object') {
    try {
      text = JSON.stringify(value)
    } catch {
      text = String(value)
    }
  } else text = String(value)

  if (text.length <= SPREADSHEET_MAX_CELL_CHARACTERS) return text
  return `${text.slice(0, SPREADSHEET_MAX_CELL_CHARACTERS)}…`
}

function uniqueReasons(reasons: string[]): string[] {
  return [...new Set(reasons)]
}

async function fetchSpreadsheetBuffer(url: string): Promise<ArrayBuffer> {
  const response = await fetch(url)
  if (!response.ok) throw new Error(`表格文件读取失败（HTTP ${response.status}）`)

  const contentLength = response.headers.get('Content-Length')
  if (contentLength) {
    const declaredLength = Number(contentLength)
    if (Number.isFinite(declaredLength)) assertSourceSize(declaredLength)
  }

  if (!response.body) {
    const buffer = await response.arrayBuffer()
    assertSourceSize(buffer.byteLength)
    return buffer
  }

  const reader = response.body.getReader()
  const chunks: Uint8Array[] = []
  let totalBytes = 0
  while (true) {
    const { done, value } = await reader.read()
    if (done) break
    totalBytes += value.byteLength
    if (totalBytes > SPREADSHEET_PREVIEW_MAX_SOURCE_BYTES) {
      await reader.cancel()
      throw new Error('表格文件超过 100 MB 的在线预览上限。')
    }
    chunks.push(value)
  }

  const bytes = new Uint8Array(totalBytes)
  let offset = 0
  for (const chunk of chunks) {
    bytes.set(chunk, offset)
    offset += chunk.byteLength
  }
  return bytes.buffer
}

function assertSourceSize(size: number): void {
  if (!Number.isFinite(size) || size < 0) throw new Error('表格文件大小无效。')
  if (size > SPREADSHEET_PREVIEW_MAX_SOURCE_BYTES) throw new Error('表格文件超过 100 MB 的在线预览上限。')
}

function spreadsheetErrorMessage(cause: unknown): string {
  const message = cause instanceof Error ? cause.message : String(cause || '')
  return message ? `表格解析失败：${message}` : '表格解析失败。'
}

function postFailure(error: string): void {
  workerScope.postMessage({ type: 'error', error })
}
