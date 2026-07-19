import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { SQLITE_PREVIEW_MAX_SOURCE_BYTES } from './sqlite-protocol'
import SqlitePreviewPlugin from './SqlitePreviewPlugin'

type PostedRequest = { type: string; requestId: number; [key: string]: unknown }

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

  lastRequest(type?: string): PostedRequest | undefined {
    const requests = this.postMessage.mock.calls.map(([request]) => request as PostedRequest).filter((request) => !type || request.type === type)
    return requests[requests.length - 1]
  }
}

const originalWorker = globalThis.Worker

beforeEach(() => {
  MockWorker.instances = []
  Object.defineProperty(globalThis, 'Worker', { configurable: true, writable: true, value: MockWorker })
})

afterEach(() => {
  cleanup()
  Object.defineProperty(globalThis, 'Worker', { configurable: true, writable: true, value: originalWorker })
})

function openUsersDatabase(worker: MockWorker) {
  const request = worker.lastRequest('open')
  expect(request).toBeDefined()
  act(() => worker.emit({
    type: 'success',
    requestId: request!.requestId,
    operation: 'open',
    result: {
      relations: [{
        name: 'users',
        type: 'table',
        columns: [
          { name: 'id', declaredType: 'INTEGER', notNull: true, primaryKey: true, hidden: false },
          { name: 'name', declaredType: 'TEXT', notNull: false, primaryKey: false, hidden: false },
        ],
      }],
    },
  }))
}

describe('SqlitePreviewPlugin', () => {
  it('browses schema, searches rows and paginates without exposing editable cells', async () => {
    render(<SqlitePreviewPlugin fileName="catalog.sqlite" mimeType="application/vnd.sqlite3" size={4096} url="/objects/catalog.sqlite" />)
    const worker = MockWorker.instances[0]
    expect(worker.options).toEqual({ type: 'module' })
    expect(worker.lastRequest('open')).toMatchObject({ url: '/objects/catalog.sqlite', sourceSize: 4096 })
    openUsersDatabase(worker)

    await waitFor(() => expect(worker.lastRequest('browse')).toMatchObject({ relation: 'users', page: 1, pageSize: 50, search: '' }))
    const firstBrowse = worker.lastRequest('browse')!
    act(() => worker.emit({
      type: 'success', requestId: firstBrowse.requestId, operation: 'browse',
      result: { columns: ['id', 'name'], rows: [[1, 'Ada']], truncated: false, page: 1, pageSize: 50, totalRows: 60 },
    }))

    expect(await screen.findByTestId('sqlite-preview')).toHaveAttribute('data-viewer-plugin', 'sqlite')
    expect(screen.getByText('users')).toBeInTheDocument()
    expect(screen.getByText('INTEGER · PK · NOT NULL')).toBeInTheDocument()
    expect(screen.getByText('Ada')).toBeInTheDocument()
    expect(screen.queryByRole('textbox', { name: /编辑/ })).not.toBeInTheDocument()
    expect(screen.getByTestId('sqlite-schema-sidebar')).toHaveClass('w-full', 'sm:w-56')
    expect(screen.getByTestId('sqlite-table-search')).toHaveClass('min-w-0')

    fireEvent.change(screen.getByRole('searchbox', { name: '搜索当前数据表' }), { target: { value: 'Ada' } })
    await waitFor(() => expect(worker.lastRequest('browse')).toMatchObject({ search: 'Ada', page: 1 }), { timeout: 1_500 })
    const searchBrowse = worker.lastRequest('browse')!
    act(() => worker.emit({
      type: 'success', requestId: searchBrowse.requestId, operation: 'browse',
      result: { columns: ['id', 'name'], rows: [[1, 'Ada']], truncated: false, page: 1, pageSize: 50, totalRows: 1 },
    }))
    expect(await screen.findByText('1 行匹配')).toBeInTheDocument()
  })

  it('runs validated read-only SQL and renders the bounded result', async () => {
    render(<SqlitePreviewPlugin fileName="catalog.db" mimeType="application/x-sqlite3" size={4096} url="/objects/catalog.db" />)
    const worker = MockWorker.instances[0]
    openUsersDatabase(worker)
    await screen.findByTestId('sqlite-preview')

    fireEvent.click(screen.getByRole('tab', { name: '只读 SQL' }))
    const editor = screen.getByRole('textbox', { name: '只读 SQL' })
    fireEvent.change(editor, { target: { value: 'DELETE FROM users' } })
    fireEvent.click(screen.getByRole('button', { name: '执行' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('不允许执行 DELETE')
    expect(worker.lastRequest('query')).toBeUndefined()

    fireEvent.change(editor, { target: { value: 'SELECT payload FROM users' } })
    fireEvent.click(screen.getByRole('button', { name: '执行' }))
    const query = worker.lastRequest('query')
    expect(query).toMatchObject({ sql: 'SELECT payload FROM users' })
    act(() => worker.emit({
      type: 'success', requestId: query!.requestId, operation: 'query',
      result: { columns: ['payload'], rows: [['BLOB (128 bytes, 0x89504e47…)']], truncated: true },
    }))
    expect(await screen.findByText('BLOB (128 bytes, 0x89504e47…)')).toBeInTheDocument()
    expect(screen.getByText('结果已按在线预览上限截断。')).toBeInTheDocument()
  })

  it('does not start a Worker above the 32 MiB admission limit', () => {
    render(<SqlitePreviewPlugin fileName="large.sqlite" mimeType="application/vnd.sqlite3" size={SQLITE_PREVIEW_MAX_SOURCE_BYTES + 1} url="/objects/large.sqlite" />)
    expect(screen.getByText('SQLite 文件超过在线预览上限')).toBeInTheDocument()
    expect(MockWorker.instances).toHaveLength(0)
  })
})
