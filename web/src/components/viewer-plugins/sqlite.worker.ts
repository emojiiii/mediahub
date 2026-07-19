import initSqlJs from 'sql.js'
import sqliteWasmUrl from 'sql.js/dist/sql-wasm.wasm?url'

import {
  isSqliteWorkerRequest,
  SQLITE_MAX_RESULT_COLUMNS,
  SQLITE_MAX_RESULT_ROWS,
  SQLITE_PREVIEW_MAX_SOURCE_BYTES,
  type SqliteBrowseRequest,
  type SqliteBrowseResult,
  type SqliteCellValue,
  type SqliteColumn,
  type SqliteOpenRequest,
  type SqliteOpenResult,
  type SqliteQueryRequest,
  type SqliteRelation,
  type SqliteTabularResult,
  type SqliteWorkerRequest,
  type SqliteWorkerResponse,
} from './sqlite-protocol'
import { validateReadonlySql } from './SqliteReadonlySql'

type SqlValue = number | string | Uint8Array | null

type SqlStatement = {
  bind(values?: SqlValue[]): boolean
  free(): boolean
  get(): SqlValue[]
  getColumnNames(): string[]
  step(): boolean
}

type SqlDatabase = {
  close(): void
  prepare(sql: string): SqlStatement
  run(sql: string): SqlDatabase
}

type WorkerScope = {
  onmessage: ((event: MessageEvent<unknown>) => void) | null
  postMessage(message: SqliteWorkerResponse): void
}

const workerScope = self as unknown as WorkerScope
const MAX_CELL_TEXT_LENGTH = 16_384

let database: SqlDatabase | null = null
let relations = new Map<string, SqliteRelation>()
let sqlitePromise: ReturnType<typeof initSqlJs> | undefined
let requestQueue = Promise.resolve()

workerScope.onmessage = (event) => {
  requestQueue = requestQueue.then(() => handleMessage(event.data))
}

async function handleMessage(message: unknown): Promise<void> {
  if (!isSqliteWorkerRequest(message)) {
    workerScope.postMessage({ type: 'error', requestId: 0, operation: 'open', error: 'SQLite 预览请求无效。' })
    return
  }

  try {
    const result = await executeRequest(message)
    workerScope.postMessage({ type: 'success', requestId: message.requestId, operation: message.type, result })
  } catch (cause) {
    workerScope.postMessage({
      type: 'error',
      requestId: message.requestId,
      operation: message.type,
      error: sqliteErrorMessage(cause),
    })
  }
}

async function executeRequest(request: SqliteWorkerRequest): Promise<SqliteOpenResult | SqliteBrowseResult | SqliteTabularResult> {
  if (request.type === 'open') return openDatabase(request)
  if (!database) throw new Error('数据库尚未加载。')
  if (request.type === 'browse') return browseRelation(database, request)
  return executeReadonlyQuery(database, request)
}

async function openDatabase(request: SqliteOpenRequest): Promise<SqliteOpenResult> {
  assertSourceSize(request.sourceSize)
  const bytes = await fetchDatabase(request.url)
  const SQL = await getSqlite()
  database?.close()
  const nextDatabase = new SQL.Database(bytes) as unknown as SqlDatabase

  try {
    nextDatabase.run('PRAGMA query_only = ON')
    nextDatabase.run('PRAGMA trusted_schema = OFF')
    const nextRelations = readRelations(nextDatabase)
    database = nextDatabase
    relations = new Map(nextRelations.map((relation) => [relation.name, relation]))
    return { relations: nextRelations }
  } catch (cause) {
    nextDatabase.close()
    throw cause
  }
}

function readRelations(db: SqlDatabase): SqliteRelation[] {
  const rows = executeRaw(
    db,
    "SELECT name, type FROM sqlite_schema WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY type, name",
    [],
    10_000,
    2,
  ).rows

  return rows.map((row) => {
    const name = String(row[0] ?? '')
    const type = row[1] === 'view' ? 'view' : 'table'
    return { name, type, columns: readColumns(db, name) }
  })
}

function readColumns(db: SqlDatabase, relation: string): SqliteColumn[] {
  let rows: SqlValue[][]
  try {
    rows = executeRaw(db, `PRAGMA table_xinfo(${quoteIdentifier(relation)})`, [], 1_000, 7).rows
  } catch {
    rows = executeRaw(db, `PRAGMA table_info(${quoteIdentifier(relation)})`, [], 1_000, 6).rows
  }
  return rows.map((row) => ({
    name: String(row[1] ?? ''),
    declaredType: String(row[2] ?? ''),
    notNull: Number(row[3]) === 1,
    primaryKey: Number(row[5]) > 0,
    hidden: Number(row[6]) > 0,
  }))
}

function browseRelation(db: SqlDatabase, request: SqliteBrowseRequest): SqliteBrowseResult {
  const relation = relations.get(request.relation)
  if (!relation) throw new Error('选择的数据表不存在。')
  const pageSize = clampInteger(request.pageSize, 10, 100)
  const page = Math.max(1, Math.trunc(request.page) || 1)
  const visibleColumns = relation.columns.filter((column) => !column.hidden).slice(0, SQLITE_MAX_RESULT_COLUMNS)
  const search = request.search.trim()
  const where = search && visibleColumns.length > 0
    ? ` WHERE (${visibleColumns.map((column) => `CAST(${quoteIdentifier(column.name)} AS TEXT) LIKE ? ESCAPE '\\' COLLATE NOCASE`).join(' OR ')})`
    : ''
  const escapedSearch = `%${escapeLike(search)}%`
  const params = search ? visibleColumns.map(() => escapedSearch) : []
  const source = quoteIdentifier(relation.name)
  const countResult = executeRaw(db, `SELECT COUNT(*) FROM ${source}${where}`, params, 1, 1)
  const totalRows = Math.max(0, Number(countResult.rows[0]?.[0]) || 0)
  const safePage = Math.max(1, Math.min(page, Math.max(1, Math.ceil(totalRows / pageSize))))
  const offset = (safePage - 1) * pageSize
  const selection = visibleColumns.length > 0 ? visibleColumns.map((column) => quoteIdentifier(column.name)).join(', ') : '*'
  const result = executeRaw(db, `SELECT ${selection} FROM ${source}${where} LIMIT ? OFFSET ?`, [...params, pageSize, offset], pageSize, SQLITE_MAX_RESULT_COLUMNS)

  return {
    columns: result.columns,
    rows: result.rows.map((row) => row.map(normalizeCell)),
    truncated: relation.columns.filter((column) => !column.hidden).length > SQLITE_MAX_RESULT_COLUMNS,
    page: safePage,
    pageSize,
    totalRows,
  }
}

function executeReadonlyQuery(db: SqlDatabase, request: SqliteQueryRequest): SqliteTabularResult {
  const validation = validateReadonlySql(request.sql)
  if (!validation.valid) throw new Error(validation.error)
  const result = executeRaw(db, validation.sql, [], SQLITE_MAX_RESULT_ROWS + 1, SQLITE_MAX_RESULT_COLUMNS)
  const truncated = result.rows.length > SQLITE_MAX_RESULT_ROWS || result.totalColumns > SQLITE_MAX_RESULT_COLUMNS
  return {
    columns: result.columns,
    rows: result.rows.slice(0, SQLITE_MAX_RESULT_ROWS).map((row) => row.map(normalizeCell)),
    truncated,
  }
}

function executeRaw(
  db: SqlDatabase,
  sql: string,
  params: SqlValue[],
  maxRows: number,
  maxColumns: number,
): { columns: string[]; rows: SqlValue[][]; totalColumns: number } {
  const statement = db.prepare(sql)
  try {
    if (params.length > 0) statement.bind(params)
    const allColumns = statement.getColumnNames()
    const rows: SqlValue[][] = []
    while (rows.length < maxRows && statement.step()) rows.push(statement.get().slice(0, maxColumns))
    return { columns: allColumns.slice(0, maxColumns), rows, totalColumns: allColumns.length }
  } finally {
    statement.free()
  }
}

function normalizeCell(value: SqlValue): SqliteCellValue {
  if (value instanceof Uint8Array) {
    const prefix = Array.from(value.subarray(0, 12), (byte) => byte.toString(16).padStart(2, '0')).join('')
    return `BLOB (${value.byteLength} bytes${prefix ? `, 0x${prefix}${value.byteLength > 12 ? '…' : ''}` : ''})`
  }
  if (typeof value === 'string' && value.length > MAX_CELL_TEXT_LENGTH) {
    return `${value.slice(0, MAX_CELL_TEXT_LENGTH)}… [已截断，共 ${value.length} 字符]`
  }
  return value
}

async function fetchDatabase(url: string): Promise<Uint8Array> {
  const response = await fetch(url)
  if (!response.ok) throw new Error(`数据库读取失败（HTTP ${response.status}）。`)
  const contentLength = Number(response.headers.get('Content-Length'))
  if (Number.isFinite(contentLength) && contentLength >= 0) assertSourceSize(contentLength)

  if (!response.body) {
    const buffer = await response.arrayBuffer()
    assertSourceSize(buffer.byteLength)
    return new Uint8Array(buffer)
  }

  const reader = response.body.getReader()
  const chunks: Uint8Array[] = []
  let totalBytes = 0
  while (true) {
    const { done, value } = await reader.read()
    if (done) break
    totalBytes += value.byteLength
    if (totalBytes > SQLITE_PREVIEW_MAX_SOURCE_BYTES) {
      await reader.cancel()
      throw new Error('SQLite 文件超过 100 MB 的在线预览上限。')
    }
    chunks.push(value)
  }
  const bytes = new Uint8Array(totalBytes)
  let offset = 0
  for (const chunk of chunks) {
    bytes.set(chunk, offset)
    offset += chunk.byteLength
  }
  return bytes
}

function getSqlite() {
  sqlitePromise ??= initSqlJs({ locateFile: () => sqliteWasmUrl })
  return sqlitePromise
}

function assertSourceSize(size: number): void {
  if (!Number.isFinite(size) || size < 0) throw new Error('SQLite 文件大小无效。')
  if (size > SQLITE_PREVIEW_MAX_SOURCE_BYTES) throw new Error('SQLite 文件超过 100 MB 的在线预览上限。')
}

function quoteIdentifier(identifier: string): string {
  return `"${identifier.replace(/"/g, '""')}"`
}

function escapeLike(value: string): string {
  return value.replace(/\\/g, '\\\\').replace(/%/g, '\\%').replace(/_/g, '\\_')
}

function clampInteger(value: number, minimum: number, maximum: number): number {
  if (!Number.isFinite(value)) return minimum
  return Math.min(maximum, Math.max(minimum, Math.trunc(value)))
}

function sqliteErrorMessage(cause: unknown): string {
  const message = cause instanceof Error ? cause.message : String(cause || '')
  if (/not a database|file is encrypted/i.test(message)) return '文件不是有效的 SQLite 数据库，或数据库已加密。'
  if (/malformed/i.test(message)) return 'SQLite 数据库已损坏，无法读取。'
  return message || 'SQLite 数据库读取失败。'
}
