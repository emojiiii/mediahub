import { Button } from '@heroui/react'
import { ChevronLeft, ChevronRight, FileSpreadsheet, RefreshCw, Search } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'

import {
  SPREADSHEET_PREVIEW_MAX_SOURCE_BYTES,
  type SpreadsheetParseRequest,
  type SpreadsheetParseResult,
  type SpreadsheetSheet,
  type SpreadsheetWorkerResponse,
} from './spreadsheet-protocol'
import { formatPreviewLimit, ViewerLoading, ViewerNotice, type ViewerFileProps } from './ViewerShared'

export const SPREADSHEET_PARSE_TIMEOUT_MS = 20_000
const DEFAULT_PAGE_SIZE = 50

type SpreadsheetPreviewState =
  | { status: 'loading' }
  | { status: 'success'; result: SpreadsheetParseResult }
  | { status: 'error'; message: string }

export default function SpreadsheetPreviewPlugin({ size, url }: ViewerFileProps) {
  const [attempt, setAttempt] = useState(0)
  const [state, setState] = useState<SpreadsheetPreviewState>({ status: 'loading' })
  const exceedsLimit = size > SPREADSHEET_PREVIEW_MAX_SOURCE_BYTES

  useEffect(() => {
    if (exceedsLimit) return

    let worker: Worker | null = null
    let timeoutId: number | undefined
    let settled = false
    setState({ status: 'loading' })

    const terminate = () => {
      if (!worker) return
      worker.terminate()
      worker = null
    }
    const finish = () => {
      if (timeoutId !== undefined) window.clearTimeout(timeoutId)
      timeoutId = undefined
      terminate()
    }
    const fail = (message: string) => {
      if (settled) return
      settled = true
      finish()
      setState({ status: 'error', message })
    }
    const succeed = (result: SpreadsheetParseResult) => {
      if (settled) return
      settled = true
      finish()
      setState({ status: 'success', result })
    }

    try {
      worker = new Worker(new URL('./spreadsheet.worker.ts', import.meta.url), { type: 'module' })
      worker.onmessage = (event: MessageEvent<SpreadsheetWorkerResponse>) => {
        const response = event.data
        if (!response || (response.type !== 'success' && response.type !== 'error')) {
          fail('表格查看器返回了无法识别的响应')
          return
        }
        if (response.type === 'success') succeed(response.result)
        else fail(response.error || '表格解析失败')
      }
      worker.onerror = (event) => fail(event.message || '表格解析失败')
      worker.onmessageerror = () => fail('表格查看器返回的数据无法读取')
      timeoutId = window.setTimeout(() => fail('表格解析超时，请重试'), SPREADSHEET_PARSE_TIMEOUT_MS)
      const request: SpreadsheetParseRequest = { type: 'parse', url, sourceSize: size }
      worker.postMessage(request)
    } catch (cause) {
      fail(cause instanceof Error ? cause.message : '无法启动表格查看器')
    }

    return () => {
      settled = true
      finish()
    }
  }, [attempt, exceedsLimit, size, url])

  if (exceedsLimit) {
    return <ViewerNotice
      title="表格超过在线预览上限"
      description={`表格预览最多处理 ${formatPreviewLimit(SPREADSHEET_PREVIEW_MAX_SOURCE_BYTES)}，请在新窗口打开或下载后查看。`}
    />
  }
  if (state.status === 'loading') return <ViewerLoading label="正在解析表格" />
  if (state.status === 'error') return <SpreadsheetError message={state.message} onRetry={() => setAttempt((value) => value + 1)} />
  if (state.result.sheets.length === 0) return <ViewerNotice title="工作簿中没有可显示的工作表" description="文件可能为空，或使用了当前解析器不支持的格式。" />
  return <SpreadsheetWorkbook result={state.result} />
}

function SpreadsheetError({ message, onRetry }: { message: string; onRetry: () => void }) {
  return <div className="flex h-full min-h-0 w-full flex-col bg-[#111317]">
    <div className="min-h-0 flex-1"><ViewerNotice title="表格解析失败" description={message} /></div>
    <div className="flex shrink-0 justify-center border-t border-white/10 bg-[#191c21] px-4 py-3">
      <Button variant="secondary" size="sm" onClick={onRetry}><RefreshCw className="size-4" />重试</Button>
    </div>
  </div>
}

export function SpreadsheetWorkbook({ result }: { result: SpreadsheetParseResult }) {
  const [sheetIndex, setSheetIndex] = useState(0)
  const [search, setSearch] = useState('')
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(DEFAULT_PAGE_SIZE)
  const sheet = result.sheets[Math.min(sheetIndex, result.sheets.length - 1)]
  const normalizedSearch = search.trim().toLocaleLowerCase()
  const searchResult = useMemo(() => filterRows(sheet, normalizedSearch), [normalizedSearch, sheet])
  const pageCount = Math.max(1, Math.ceil(searchResult.rows.length / pageSize))
  const safePage = Math.min(page, pageCount)
  const visibleRows = searchResult.rows.slice((safePage - 1) * pageSize, safePage * pageSize)

  useEffect(() => setPage(1), [normalizedSearch, pageSize, sheetIndex])

  const selectSheet = (nextIndex: number) => {
    setSheetIndex(nextIndex)
    setSearch('')
  }

  return <div data-testid="spreadsheet-preview" data-viewer-plugin="spreadsheet" className="flex h-full min-h-0 w-full flex-col bg-white text-[#1f2937]">
    <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-[#d9dee7] bg-[#f3f5f8] px-3 py-2 text-xs">
      <FileSpreadsheet className="size-4 shrink-0 text-[#15803d]" />
      <label className="flex min-w-0 items-center gap-2 font-semibold text-[#334155]">
        <span className="sr-only">工作表</span>
        <select
          aria-label="工作表"
          className="h-8 max-w-52 rounded-md border border-[#cbd5e1] bg-white px-2 text-xs font-semibold text-[#334155]"
          value={sheetIndex}
          onChange={(event) => selectSheet(Number(event.target.value))}
        >
          {result.sheets.map((item, index) => <option key={`${item.name}-${index}`} value={index}>{item.name}</option>)}
        </select>
      </label>
      <label className="relative ml-auto min-w-44 flex-1 sm:max-w-72">
        <span className="sr-only">搜索表格</span>
        <Search className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-[#64748b]" />
        <input
          aria-label="搜索表格"
          className="h-8 w-full rounded-md border border-[#cbd5e1] bg-white pl-8 pr-2 text-xs text-[#334155] placeholder:text-[#94a3b8]"
          placeholder="搜索当前工作表"
          type="search"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
        />
      </label>
      <span aria-live="polite" className="shrink-0 tabular-nums text-[#64748b]">
        {normalizedSearch
          ? `${searchResult.cellMatches.toLocaleString('zh-CN')} 个单元格匹配`
          : `${sheet.rows.length.toLocaleString('zh-CN')} 行 × ${sheet.columnLabels.length.toLocaleString('zh-CN')} 列`}
      </span>
    </div>

    <SpreadsheetTable sheet={sheet} rows={visibleRows} normalizedSearch={normalizedSearch} />

    {result.truncated && <p className="shrink-0 border-t border-[#e6d7a8] bg-[#fffbea] px-3 py-1.5 text-[11px] leading-5 text-[#854d0e]" title={result.truncationReasons.join('；')}>
      数据已截断：{result.truncationReasons.join('；')}
    </p>}

    <footer className="flex min-h-11 shrink-0 flex-wrap items-center justify-between gap-2 border-t border-[#d9dee7] bg-[#f8fafc] px-3 py-2 text-xs text-[#64748b]">
      <span>{normalizedSearch ? `${searchResult.rows.length.toLocaleString('zh-CN')} 个匹配行` : `已加载 ${sheet.rows.length.toLocaleString('zh-CN')} / ${sheet.sourceRowCount.toLocaleString('zh-CN')} 行`}</span>
      <div className="flex items-center gap-2">
        <label className="flex items-center gap-1.5">
          <span>每页</span>
          <select
            aria-label="每页行数"
            className="h-8 rounded-md border border-[#cbd5e1] bg-white px-2 text-xs text-[#334155]"
            value={pageSize}
            onChange={(event) => setPageSize(Number(event.target.value))}
          >
            {[25, 50, 100].map((value) => <option key={value} value={value}>{value}</option>)}
          </select>
        </label>
        <Button isIconOnly aria-label="上一页" variant="ghost" size="sm" isDisabled={safePage <= 1} onClick={() => setPage(Math.max(1, safePage - 1))}>
          <ChevronLeft className="size-4" />
        </Button>
        <span className="min-w-20 text-center tabular-nums">第 {safePage} / {pageCount} 页</span>
        <Button isIconOnly aria-label="下一页" variant="ghost" size="sm" isDisabled={safePage >= pageCount} onClick={() => setPage(Math.min(pageCount, safePage + 1))}>
          <ChevronRight className="size-4" />
        </Button>
      </div>
    </footer>
  </div>
}

type FilteredRow = {
  cells: string[]
  sourceIndex: number
}

function filterRows(sheet: SpreadsheetSheet, normalizedSearch: string): { rows: FilteredRow[]; cellMatches: number } {
  let cellMatches = 0
  const rows: FilteredRow[] = []
  sheet.rows.forEach((cells, sourceIndex) => {
    if (!normalizedSearch) {
      rows.push({ cells, sourceIndex })
      return
    }
    const rowMatches = cells.reduce((count, cell) => count + (cell.toLocaleLowerCase().includes(normalizedSearch) ? 1 : 0), 0)
    if (rowMatches > 0) {
      cellMatches += rowMatches
      rows.push({ cells, sourceIndex })
    }
  })
  return { rows, cellMatches }
}

function SpreadsheetTable({ sheet, rows, normalizedSearch }: { sheet: SpreadsheetSheet; rows: FilteredRow[]; normalizedSearch: string }) {
  if (sheet.columnLabels.length === 0 || sheet.rows.length === 0) {
    return <div className="grid min-h-0 flex-1 place-items-center px-6 text-center text-xs text-[#64748b]">当前工作表没有可显示的数据</div>
  }
  if (rows.length === 0) {
    return <div className="grid min-h-0 flex-1 place-items-center px-6 text-center text-xs text-[#64748b]">没有匹配的单元格</div>
  }

  return <div data-testid="spreadsheet-scroll-region" className="min-h-0 flex-1 overflow-auto">
    <table aria-label={`${sheet.name} 数据`} className="min-w-max border-separate border-spacing-0 text-xs">
      <thead className="sticky top-0 z-20 bg-[#eef2f7] text-[#475569]">
        <tr>
          <th className="sticky left-0 z-30 min-w-14 border-b border-r border-[#d9dee7] bg-[#e5eaf1] px-2 py-2 text-right font-semibold">#</th>
          {sheet.columnLabels.map((label) => <th key={label} className="min-w-32 border-b border-r border-[#d9dee7] bg-[#eef2f7] px-3 py-2 text-left font-semibold">{label}</th>)}
        </tr>
      </thead>
      <tbody>
        {rows.map(({ cells, sourceIndex }) => <tr key={sourceIndex} className="even:bg-[#f8fafc] hover:bg-[#eff6ff]">
          <th className="sticky left-0 z-10 border-b border-r border-[#e2e8f0] bg-[#f1f5f9] px-2 py-2 text-right font-normal tabular-nums text-[#64748b]">{sheet.startRow + sourceIndex}</th>
          {sheet.columnLabels.map((label, columnIndex) => {
            const value = cells[columnIndex] ?? ''
            const matches = normalizedSearch && value.toLocaleLowerCase().includes(normalizedSearch)
            return <td
              key={label}
              className={`max-w-72 border-b border-r border-[#e2e8f0] px-3 py-2 align-top text-[#334155] ${matches ? 'bg-[#fef3c7]' : ''}`}
              title={value}
            >
              <span className="block max-w-64 truncate">{value || '\u00a0'}</span>
            </td>
          })}
        </tr>)}
      </tbody>
    </table>
  </div>
}
