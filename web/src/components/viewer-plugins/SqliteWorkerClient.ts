import {
  SQLITE_QUERY_TIMEOUT_MS,
  type SqliteBrowseRequest,
  type SqliteBrowseResult,
  type SqliteOpenRequest,
  type SqliteOpenResult,
  type SqliteQueryRequest,
  type SqliteTabularResult,
  type SqliteWorkerRequest,
  type SqliteWorkerResponse,
} from './sqlite-protocol'

export const SQLITE_OPEN_TIMEOUT_MS = 30_000

export class SqliteWorkerClientError extends Error {
  constructor(
    message: string,
    readonly code: 'request' | 'timeout' | 'worker' | 'terminated',
  ) {
    super(message)
    this.name = 'SqliteWorkerClientError'
  }
}

type PendingRequest = {
  operation: SqliteWorkerRequest['type']
  resolve(value: unknown): void
  reject(error: SqliteWorkerClientError): void
  timeoutId: number
}

export class SqliteWorkerClient {
  private readonly pending = new Map<number, PendingRequest>()
  private nextRequestId = 1
  private terminated = false

  constructor(private readonly worker: Worker = new Worker(new URL('./sqlite.worker.ts', import.meta.url), { type: 'module' })) {
    worker.onmessage = (event: MessageEvent<SqliteWorkerResponse>) => this.handleMessage(event.data)
    worker.onerror = (event) => this.failWorker(event.message || 'SQLite Worker 执行失败。')
    worker.onmessageerror = () => this.failWorker('SQLite Worker 返回了无法读取的数据。')
  }

  open(url: string, sourceSize: number): Promise<SqliteOpenResult> {
    return this.request<SqliteOpenResult>({ type: 'open', url, sourceSize }, SQLITE_OPEN_TIMEOUT_MS)
  }

  browse(request: Omit<SqliteBrowseRequest, 'type' | 'requestId'>): Promise<SqliteBrowseResult> {
    return this.request<SqliteBrowseResult>({ type: 'browse', ...request }, SQLITE_QUERY_TIMEOUT_MS)
  }

  query(sql: string): Promise<SqliteTabularResult> {
    return this.request<SqliteTabularResult>({ type: 'query', sql }, SQLITE_QUERY_TIMEOUT_MS)
  }

  terminate(reason = new SqliteWorkerClientError('SQLite Worker 已停止。', 'terminated')): void {
    if (this.terminated) return
    this.terminated = true
    this.worker.terminate()
    for (const pending of this.pending.values()) {
      window.clearTimeout(pending.timeoutId)
      pending.reject(reason)
    }
    this.pending.clear()
  }

  private request<Result>(
    request: Omit<SqliteOpenRequest, 'requestId'> | Omit<SqliteBrowseRequest, 'requestId'> | Omit<SqliteQueryRequest, 'requestId'>,
    timeoutMs: number,
  ): Promise<Result> {
    if (this.terminated) return Promise.reject(new SqliteWorkerClientError('SQLite Worker 已停止。', 'terminated'))
    const requestId = this.nextRequestId++
    return new Promise<Result>((resolve, reject) => {
      const timeoutId = window.setTimeout(() => {
        const pending = this.pending.get(requestId)
        if (!pending) return
        this.pending.delete(requestId)
        const error = new SqliteWorkerClientError(
          request.type === 'open' ? 'SQLite 数据库加载超时。' : 'SQLite 查询超过 10 秒，数据库引擎已重启。',
          'timeout',
        )
        pending.reject(error)
        this.terminate(error)
      }, timeoutMs)
      this.pending.set(requestId, {
        operation: request.type,
        resolve: (value) => resolve(value as Result),
        reject,
        timeoutId,
      })
      this.worker.postMessage({ ...request, requestId } satisfies SqliteWorkerRequest)
    })
  }

  private handleMessage(response: SqliteWorkerResponse): void {
    if (!response || (response.type !== 'success' && response.type !== 'error')) {
      this.failWorker('SQLite Worker 返回了无法识别的响应。')
      return
    }
    const pending = this.pending.get(response.requestId)
    if (!pending) return
    this.pending.delete(response.requestId)
    window.clearTimeout(pending.timeoutId)
    if (response.operation !== pending.operation) {
      pending.reject(new SqliteWorkerClientError('SQLite Worker 响应类型不匹配。', 'worker'))
      return
    }
    if (response.type === 'error') pending.reject(new SqliteWorkerClientError(response.error, 'request'))
    else pending.resolve(response.result)
  }

  private failWorker(message: string): void {
    this.terminate(new SqliteWorkerClientError(message, 'worker'))
  }
}

