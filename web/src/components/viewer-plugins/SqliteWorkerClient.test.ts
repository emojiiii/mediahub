import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { SQLITE_QUERY_TIMEOUT_MS } from './sqlite-protocol'
import { SqliteWorkerClient, SqliteWorkerClientError } from './SqliteWorkerClient'

class MockWorker {
  onerror: ((event: ErrorEvent) => unknown) | null = null
  onmessage: ((event: MessageEvent) => unknown) | null = null
  onmessageerror: ((event: MessageEvent) => unknown) | null = null
  postMessage = vi.fn()
  terminate = vi.fn()

  emit(data: unknown) {
    this.onmessage?.({ data } as MessageEvent)
  }
}

describe('SqliteWorkerClient', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => vi.useRealTimers())

  it('correlates protocol responses with open, browse and query requests', async () => {
    const worker = new MockWorker()
    const client = new SqliteWorkerClient(worker as unknown as Worker)
    const opening = client.open('/objects/catalog.sqlite', 4096)
    expect(worker.postMessage).toHaveBeenLastCalledWith({
      type: 'open', requestId: 1, url: '/objects/catalog.sqlite', sourceSize: 4096,
    })
    worker.emit({ type: 'success', requestId: 1, operation: 'open', result: { relations: [] } })
    await expect(opening).resolves.toEqual({ relations: [] })

    const browsing = client.browse({ relation: 'users', page: 2, pageSize: 25, search: 'Ada' })
    expect(worker.postMessage).toHaveBeenLastCalledWith({
      type: 'browse', requestId: 2, relation: 'users', page: 2, pageSize: 25, search: 'Ada',
    })
    worker.emit({
      type: 'success', requestId: 2, operation: 'browse',
      result: { columns: ['name'], rows: [['Ada']], truncated: false, page: 2, pageSize: 25, totalRows: 26 },
    })
    await expect(browsing).resolves.toMatchObject({ rows: [['Ada']], totalRows: 26 })

    const query = client.query('SELECT 1')
    worker.emit({ type: 'success', requestId: 3, operation: 'query', result: { columns: ['1'], rows: [[1]], truncated: false } })
    await expect(query).resolves.toMatchObject({ rows: [[1]] })
    client.terminate()
  })

  it('terminates the database Worker when a query exceeds ten seconds', async () => {
    const worker = new MockWorker()
    const client = new SqliteWorkerClient(worker as unknown as Worker)
    const query = client.query('SELECT expensive_operation()')

    vi.advanceTimersByTime(SQLITE_QUERY_TIMEOUT_MS)

    await expect(query).rejects.toMatchObject({ code: 'timeout' } satisfies Partial<SqliteWorkerClientError>)
    expect(worker.terminate).toHaveBeenCalledOnce()
    await expect(client.query('SELECT 1')).rejects.toMatchObject({ code: 'terminated' })
  })

  it('rejects malformed and mismatched Worker responses', async () => {
    const worker = new MockWorker()
    const client = new SqliteWorkerClient(worker as unknown as Worker)
    const query = client.query('SELECT 1')
    worker.emit({ type: 'success', requestId: 1, operation: 'browse', result: {} })
    await expect(query).rejects.toMatchObject({ code: 'worker' })
    client.terminate()
  })
})
