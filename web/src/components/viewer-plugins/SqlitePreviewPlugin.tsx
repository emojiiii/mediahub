import { Button, Spinner } from '@heroui/react'
import { ChevronLeft, ChevronRight, Database, Play, RefreshCw, Search, Table2 } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'

import {
  SQLITE_MAX_RESULT_COLUMNS,
  SQLITE_MAX_RESULT_ROWS,
  SQLITE_PREVIEW_MAX_SOURCE_BYTES,
  type SqliteBrowseResult,
  type SqliteOpenResult,
  type SqliteRelation,
  type SqliteTabularResult,
} from './sqlite-protocol'
import { validateReadonlySql } from './SqliteReadonlySql'
import { SqliteWorkerClient, SqliteWorkerClientError } from './SqliteWorkerClient'
import { formatPreviewLimit, ViewerLoading, ViewerNotice, type ViewerFileProps } from './ViewerShared'

type OpenState =
  | { status: 'loading' }
  | { status: 'ready'; result: SqliteOpenResult }
  | { status: 'error'; message: string }

type BrowseState =
  | { status: 'idle' | 'loading' }
  | { status: 'ready'; result: SqliteBrowseResult }
  | { status: 'error'; message: string }

type QueryState =
  | { status: 'idle' | 'loading' }
  | { status: 'ready'; result: SqliteTabularResult }
  | { status: 'error'; message: string }

const DEFAULT_SQL = 'SELECT name, type\nFROM sqlite_schema\nORDER BY type, name;'
const BROWSE_SEARCH_DEBOUNCE_MS = 300

export default function SqlitePreviewPlugin({ size, url }: ViewerFileProps) {
  const [runtimeVersion, setRuntimeVersion] = useState(0)
  const [openState, setOpenState] = useState<OpenState>({ status: 'loading' })
  const [mode, setMode] = useState<'browse' | 'query'>('browse')
  const [selectedRelation, setSelectedRelation] = useState('')
  const [browseState, setBrowseState] = useState<BrowseState>({ status: 'idle' })
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(50)
  const [searchInput, setSearchInput] = useState('')
  const [search, setSearch] = useState('')
  const [sql, setSql] = useState(DEFAULT_SQL)
  const [queryState, setQueryState] = useState<QueryState>({ status: 'idle' })
  const clientRef = useRef<SqliteWorkerClient | null>(null)
  const recoveryScheduledRef = useRef(false)
  const exceedsLimit = size > SQLITE_PREVIEW_MAX_SOURCE_BYTES

  useEffect(() => {
    if (exceedsLimit) return
    let active = true
    recoveryScheduledRef.current = false
    const client = new SqliteWorkerClient()
    clientRef.current = client
    setOpenState({ status: 'loading' })

    void client.open(url, size).then((result) => {
      if (!active) return
      setOpenState({ status: 'ready', result })
      setSelectedRelation((current) => result.relations.some((relation) => relation.name === current)
        ? current
        : (result.relations[0]?.name ?? ''))
    }).catch((cause) => {
      if (!active) return
      setOpenState({ status: 'error', message: errorMessage(cause) })
    })

    return () => {
      active = false
      if (clientRef.current === client) clientRef.current = null
      client.terminate()
    }
  }, [exceedsLimit, runtimeVersion, size, url])

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      setPage(1)
      setSearch(searchInput.trim())
    }, BROWSE_SEARCH_DEBOUNCE_MS)
    return () => window.clearTimeout(timeoutId)
  }, [searchInput])

  useEffect(() => {
    if (openState.status !== 'ready' || !selectedRelation) {
      setBrowseState({ status: 'idle' })
      return
    }
    const client = clientRef.current
    if (!client) return
    let active = true
    setBrowseState({ status: 'loading' })
    void client.browse({ relation: selectedRelation, page, pageSize, search }).then((result) => {
      if (!active) return
      setBrowseState({ status: 'ready', result })
      if (result.page !== page) setPage(result.page)
    }).catch((cause) => {
      if (!active || isTerminatedError(cause)) return
      setBrowseState({ status: 'error', message: errorMessage(cause) })
      if (isTimeoutError(cause)) restartWorker()
    })
    return () => { active = false }
  }, [openState, page, pageSize, search, selectedRelation])

  const currentRelation = useMemo(() => openState.status === 'ready'
    ? openState.result.relations.find((relation) => relation.name === selectedRelation)
    : undefined, [openState, selectedRelation])

  function restartWorker() {
    if (recoveryScheduledRef.current) return
    recoveryScheduledRef.current = true
    setRuntimeVersion((value) => value + 1)
  }

  async function runQuery() {
    const validation = validateReadonlySql(sql)
    if (!validation.valid) {
      setQueryState({ status: 'error', message: validation.error })
      return
    }
    const client = clientRef.current
    if (!client) return
    setQueryState({ status: 'loading' })
    try {
      const result = await client.query(validation.sql)
      setQueryState({ status: 'ready', result })
    } catch (cause) {
      if (isTerminatedError(cause)) return
      setQueryState({ status: 'error', message: errorMessage(cause) })
      if (isTimeoutError(cause)) restartWorker()
    }
  }

  if (exceedsLimit) return <ViewerNotice
    title="SQLite 文件超过在线预览上限"
    description={`SQLite 在线查询最多处理 ${formatPreviewLimit(SQLITE_PREVIEW_MAX_SOURCE_BYTES)}，请下载后使用本地工具查看。`}
  />
  if (openState.status === 'loading') return <ViewerLoading label="正在加载 SQLite 数据库" />
  if (openState.status === 'error') return <SqliteOpenError message={openState.message} onRetry={restartWorker} />

  return <div data-testid="sqlite-preview" data-viewer-plugin="sqlite" className="flex h-full min-h-0 w-full min-w-0 flex-col bg-white text-[#1f2937]">
    <header className="flex min-h-12 shrink-0 flex-wrap items-center gap-2 border-b border-[#d9dee7] bg-[#f6f7f9] px-3">
      <span className="mr-1 flex items-center gap-2 text-xs font-semibold text-[#334155]"><Database className="size-4 text-[#2563eb]" />SQLite</span>
      <div className="flex h-8 items-center rounded-md border border-[#d9dee7] bg-white p-0.5" role="tablist" aria-label="SQLite 预览模式">
        <ModeTab selected={mode === 'browse'} onClick={() => setMode('browse')}>数据浏览</ModeTab>
        <ModeTab selected={mode === 'query'} onClick={() => setMode('query')}>只读 SQL</ModeTab>
      </div>
      <span className="ml-auto text-[11px] tabular-nums text-[#64748b]">{openState.result.relations.length} 个表或视图</span>
    </header>

    {mode === 'browse'
      ? <BrowsePanel
          relations={openState.result.relations}
          currentRelation={currentRelation}
          selectedRelation={selectedRelation}
          state={browseState}
          page={page}
          pageSize={pageSize}
          search={searchInput}
          onPageChange={setPage}
          onPageSizeChange={(value) => { setPage(1); setPageSize(value) }}
          onSearchChange={setSearchInput}
          onSelectRelation={(relation) => {
            setSelectedRelation(relation)
            setPage(1)
            setSearchInput('')
            setSearch('')
          }}
        />
      : <QueryPanel sql={sql} state={queryState} onRun={() => void runQuery()} onSqlChange={setSql} />}
  </div>
}

function ModeTab({ children, selected, onClick }: { children: React.ReactNode; selected: boolean; onClick: () => void }) {
  return <button type="button" role="tab" aria-selected={selected} className={`h-7 rounded px-3 text-xs font-medium transition ${selected ? 'bg-[#e7efff] text-[#1d4ed8]' : 'text-[#64748b] hover:bg-[#f1f5f9]'}`} onClick={onClick}>{children}</button>
}

function BrowsePanel({
  relations,
  currentRelation,
  selectedRelation,
  state,
  page,
  pageSize,
  search,
  onPageChange,
  onPageSizeChange,
  onSearchChange,
  onSelectRelation,
}: {
  relations: SqliteRelation[]
  currentRelation?: SqliteRelation
  selectedRelation: string
  state: BrowseState
  page: number
  pageSize: number
  search: string
  onPageChange: (page: number) => void
  onPageSizeChange: (pageSize: number) => void
  onSearchChange: (search: string) => void
  onSelectRelation: (relation: string) => void
}) {
  if (relations.length === 0) return <ViewerNotice title="数据库中没有可浏览的数据表" description="该数据库不包含普通表或视图。" />
  const totalPages = state.status === 'ready' ? Math.max(1, Math.ceil(state.result.totalRows / state.result.pageSize)) : 1

  return <div className="flex min-h-0 flex-1 flex-col sm:flex-row">
    <aside data-testid="sqlite-schema-sidebar" className="flex max-h-40 w-full shrink-0 flex-col border-b border-[#d9dee7] bg-[#f8fafc] sm:max-h-none sm:w-56 sm:border-b-0 sm:border-r">
      <div className="shrink-0 border-b border-[#e2e8f0] px-3 py-2 text-[11px] font-semibold uppercase text-[#64748b]">数据表</div>
      <nav aria-label="SQLite 数据表" className="flex max-h-16 shrink-0 gap-1 overflow-auto border-b border-[#e2e8f0] p-1.5 sm:block sm:max-h-[45%]">
        {relations.map((relation) => <button
          key={relation.name}
          type="button"
          title={relation.name}
          className={`flex h-8 w-44 shrink-0 min-w-0 items-center gap-2 rounded px-2 text-left text-xs sm:w-full ${selectedRelation === relation.name ? 'bg-[#e7efff] font-medium text-[#1d4ed8]' : 'text-[#475569] hover:bg-[#eef2f7]'}`}
          onClick={() => onSelectRelation(relation.name)}
        ><Table2 className="size-3.5 shrink-0" /><span className="min-w-0 flex-1 truncate">{relation.name}</span><span className="text-[9px] uppercase opacity-60">{relation.type === 'view' ? 'view' : ''}</span></button>)}
      </nav>
      <div className="shrink-0 px-3 py-2 text-[11px] font-semibold uppercase text-[#64748b]">字段</div>
      <ul aria-label="SQLite 字段" className="flex min-h-0 flex-1 gap-2 overflow-auto px-2 pb-2 sm:block">
        {currentRelation?.columns.map((column, index) => <li key={`${column.name}-${index}`} className="min-w-40 border-b border-[#e8ecf2] px-1 py-2 text-[11px] last:border-0 sm:min-w-0" title={`${column.name} ${column.declaredType}`}>
          <span className="block truncate font-mono font-medium text-[#334155]">{column.name}</span>
          <span className="mt-0.5 block truncate text-[10px] text-[#8491a3]">{column.declaredType || '无类型'}{column.primaryKey ? ' · PK' : ''}{column.notNull ? ' · NOT NULL' : ''}{column.hidden ? ' · hidden' : ''}</span>
        </li>)}
      </ul>
    </aside>

    <section className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div className="flex min-h-12 shrink-0 flex-wrap items-center gap-3 border-b border-[#d9dee7] px-3 py-2">
        <div data-testid="sqlite-table-search" className="relative min-w-0 basis-40 sm:max-w-md sm:flex-1">
          <Search className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-[#94a3b8]" />
          <input aria-label="搜索当前数据表" type="search" value={search} placeholder="搜索当前表" className="h-8 w-full rounded-md border border-[#cbd5e1] bg-white pl-8 pr-3 text-xs outline-none focus:border-[#2563eb] focus:ring-2 focus:ring-[#2563eb]/10" onChange={(event) => onSearchChange(event.target.value)} />
        </div>
        {state.status === 'ready' && <span className="text-[11px] tabular-nums text-[#64748b]">{state.result.totalRows} 行{search.trim() ? '匹配' : ''}</span>}
      </div>

      <div className="min-h-0 flex-1 bg-white">
        {state.status === 'loading' && <InlineLoading label="正在读取数据" />}
        {state.status === 'error' && <ViewerNotice title="数据表读取失败" description={state.message} />}
        {state.status === 'ready' && <ResultGrid result={state.result} emptyLabel={search.trim() ? '没有匹配的数据' : '该数据表为空'} />}
      </div>

      <footer className="flex min-h-11 shrink-0 flex-wrap items-center justify-between gap-3 border-t border-[#d9dee7] bg-[#f8fafc] px-3 py-2 text-[11px] text-[#64748b]">
        <label className="flex items-center gap-2">每页
          <select aria-label="SQLite 每页数量" className="h-7 rounded border border-[#cbd5e1] bg-white px-2 text-xs text-[#334155]" value={pageSize} onChange={(event) => onPageSizeChange(Number(event.target.value))}>
            <option value={25}>25</option><option value={50}>50</option><option value={100}>100</option>
          </select>
        </label>
        <div className="flex items-center gap-2">
          <button type="button" aria-label="上一页" className="grid size-7 place-items-center rounded border border-[#cbd5e1] bg-white disabled:opacity-40" disabled={page <= 1 || state.status === 'loading'} onClick={() => onPageChange(page - 1)}><ChevronLeft className="size-3.5" /></button>
          <span className="min-w-20 text-center tabular-nums">第 {page} / {totalPages} 页</span>
          <button type="button" aria-label="下一页" className="grid size-7 place-items-center rounded border border-[#cbd5e1] bg-white disabled:opacity-40" disabled={page >= totalPages || state.status === 'loading'} onClick={() => onPageChange(page + 1)}><ChevronRight className="size-3.5" /></button>
        </div>
      </footer>
    </section>
  </div>
}

function QueryPanel({ sql, state, onRun, onSqlChange }: { sql: string; state: QueryState; onRun: () => void; onSqlChange: (sql: string) => void }) {
  return <section className="flex min-h-0 flex-1 flex-col">
    <div className="shrink-0 border-b border-[#d9dee7] bg-[#f8fafc] p-3">
      <textarea
        aria-label="只读 SQL"
        spellCheck={false}
        value={sql}
        className="h-28 w-full resize-y rounded-md border border-[#cbd5e1] bg-white p-3 font-mono text-xs leading-5 text-[#1e293b] outline-none focus:border-[#2563eb] focus:ring-2 focus:ring-[#2563eb]/10"
        onChange={(event) => onSqlChange(event.target.value)}
        onKeyDown={(event) => {
          if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') {
            event.preventDefault()
            onRun()
          }
        }}
      />
      <div className="mt-2 flex items-center justify-between gap-3">
        <span className="text-[11px] text-[#64748b]">最多返回 {SQLITE_MAX_RESULT_ROWS} 行、{SQLITE_MAX_RESULT_COLUMNS} 列</span>
        <Button variant="primary" size="sm" isDisabled={state.status === 'loading'} onClick={onRun}>{state.status === 'loading' ? <Spinner size="sm" /> : <Play className="size-3.5" />}执行</Button>
      </div>
      {state.status === 'error' && <p role="alert" className="mt-2 rounded border border-[#fecaca] bg-[#fff1f2] px-3 py-2 text-xs text-[#b42318]">{state.message}</p>}
    </div>
    <div className="min-h-0 flex-1">
      {state.status === 'idle' && <ViewerNotice title="等待执行 SQL" description="输入只读查询后查看结果。" />}
      {state.status === 'loading' && <InlineLoading label="正在执行查询" />}
      {state.status === 'ready' && <ResultGrid result={state.result} emptyLabel="查询没有返回数据" />}
    </div>
  </section>
}

function ResultGrid({ result, emptyLabel }: { result: SqliteTabularResult; emptyLabel: string }) {
  if (result.rows.length === 0) return <ViewerNotice title={emptyLabel} description={result.columns.length > 0 ? `结果包含 ${result.columns.length} 个字段。` : '该语句没有返回结果集。'} />
  return <div data-testid="sqlite-result-grid" className="flex h-full min-h-0 flex-col">
    <div className="min-h-0 flex-1 overflow-auto">
      <table className="min-w-full border-separate border-spacing-0 text-left font-mono text-[11px]">
        <thead className="sticky top-0 z-[1] bg-[#eef2f7] text-[#475569]">
          <tr>{result.columns.map((column, index) => <th key={`${column}-${index}`} className="max-w-80 border-b border-r border-[#d9dee7] px-3 py-2 font-semibold last:border-r-0" title={column}>{column || `(列 ${index + 1})`}</th>)}</tr>
        </thead>
        <tbody>{result.rows.map((row, rowIndex) => <tr key={rowIndex} className="odd:bg-white even:bg-[#fafbfc] hover:bg-[#eff6ff]">
          {result.columns.map((_, columnIndex) => {
            const value = row[columnIndex]
            return <td key={columnIndex} className="max-w-80 border-b border-r border-[#e5e7eb] px-3 py-2 text-[#334155] last:border-r-0" title={value == null ? 'NULL' : String(value)}><span className={`block max-w-80 truncate ${value == null ? 'italic text-[#94a3b8]' : ''}`}>{value == null ? 'NULL' : String(value)}</span></td>
          })}
        </tr>)}</tbody>
      </table>
    </div>
    {result.truncated && <p className="shrink-0 border-t border-[#e6d7a8] bg-[#fffbea] px-3 py-2 text-[11px] text-[#854d0e]">结果已按在线预览上限截断。</p>}
  </div>
}

function InlineLoading({ label }: { label: string }) {
  return <div className="grid h-full place-items-center text-xs text-[#64748b]"><span className="flex items-center gap-2"><Spinner aria-label={label} size="sm" />{label}</span></div>
}

function SqliteOpenError({ message, onRetry }: { message: string; onRetry: () => void }) {
  return <div className="flex h-full min-h-0 flex-col bg-[#111317]">
    <div className="min-h-0 flex-1"><ViewerNotice title="SQLite 数据库读取失败" description={message} /></div>
    <div className="flex shrink-0 justify-center border-t border-white/10 bg-[#191c21] px-4 py-3"><Button variant="secondary" size="sm" onClick={onRetry}><RefreshCw className="size-4" />重试</Button></div>
  </div>
}

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : 'SQLite 预览失败。'
}

function isTimeoutError(cause: unknown): boolean {
  return cause instanceof SqliteWorkerClientError && cause.code === 'timeout'
}

function isTerminatedError(cause: unknown): boolean {
  return cause instanceof SqliteWorkerClientError && cause.code === 'terminated'
}
