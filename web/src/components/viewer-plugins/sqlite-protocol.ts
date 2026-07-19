import { BUFFERED_PREVIEW_MAX_BYTES } from './BufferedPreviewPolicy'

export const SQLITE_PREVIEW_MAX_SOURCE_BYTES = BUFFERED_PREVIEW_MAX_BYTES
export const SQLITE_QUERY_TIMEOUT_MS = 10_000
export const SQLITE_MAX_RESULT_ROWS = 500
export const SQLITE_MAX_RESULT_COLUMNS = 64

export type SqliteCellValue = string | number | null

export type SqliteColumn = {
  name: string
  declaredType: string
  notNull: boolean
  primaryKey: boolean
  hidden: boolean
}

export type SqliteRelation = {
  name: string
  type: 'table' | 'view'
  columns: SqliteColumn[]
}

export type SqliteOpenResult = {
  relations: SqliteRelation[]
}

export type SqliteTabularResult = {
  columns: string[]
  rows: SqliteCellValue[][]
  truncated: boolean
}

export type SqliteBrowseResult = SqliteTabularResult & {
  page: number
  pageSize: number
  totalRows: number
}

export type SqliteOpenRequest = {
  type: 'open'
  requestId: number
  url: string
  sourceSize: number
}

export type SqliteBrowseRequest = {
  type: 'browse'
  requestId: number
  relation: string
  page: number
  pageSize: number
  search: string
}

export type SqliteQueryRequest = {
  type: 'query'
  requestId: number
  sql: string
}

export type SqliteWorkerRequest = SqliteOpenRequest | SqliteBrowseRequest | SqliteQueryRequest

export type SqliteWorkerSuccess = {
  type: 'success'
  requestId: number
  operation: SqliteWorkerRequest['type']
  result: SqliteOpenResult | SqliteBrowseResult | SqliteTabularResult
}

export type SqliteWorkerFailure = {
  type: 'error'
  requestId: number
  operation: SqliteWorkerRequest['type']
  error: string
}

export type SqliteWorkerResponse = SqliteWorkerSuccess | SqliteWorkerFailure

export function isSqliteWorkerRequest(value: unknown): value is SqliteWorkerRequest {
  if (!value || typeof value !== 'object') return false
  const request = value as Partial<SqliteWorkerRequest>
  if (!Number.isSafeInteger(request.requestId) || Number(request.requestId) < 0) return false
  if (request.type === 'open') {
    return typeof request.url === 'string' && typeof request.sourceSize === 'number'
  }
  if (request.type === 'browse') {
    return typeof request.relation === 'string'
      && typeof request.page === 'number'
      && typeof request.pageSize === 'number'
      && typeof request.search === 'string'
  }
  return request.type === 'query' && typeof request.sql === 'string'
}
