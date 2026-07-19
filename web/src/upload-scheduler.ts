export const MIN_UPLOAD_CONCURRENCY = 1
export const MAX_UPLOAD_CONCURRENCY = 12
export const DEFAULT_UPLOAD_CONCURRENCY = 4

export type UploadScheduleState = 'pending' | 'active' | 'succeeded' | 'failed' | 'cancelled'

export type UploadRunContext = {
  id: string
  attempt: number
  signal: AbortSignal
}

export type UploadRunner<Input, Result> = (
  input: Input,
  context: UploadRunContext,
) => Promise<Result>

export type ScheduledUpload<Input, Result> = {
  id: string
  input: Input
  state: UploadScheduleState
  attempts: number
  enqueuedAt: number
  startedAt?: number
  completedAt?: number
  result?: Result
  error?: unknown
}

export type UploadSchedulerSnapshot<Input, Result> = {
  concurrency: number
  activeCount: number
  pendingCount: number
  tasks: ReadonlyArray<Readonly<ScheduledUpload<Input, Result>>>
}

export type UploadSchedulerOptions = {
  concurrency?: number
  maxRetries?: number
  baseRetryDelayMs?: number
  maxRetryDelayMs?: number
  jitterRatio?: number
  random?: () => number
  delay?: (milliseconds: number, signal: AbortSignal) => Promise<void>
  isTransientError?: (error: unknown) => boolean
}

const TRANSIENT_HTTP_STATUSES = new Set([429, 502, 503, 504])

export function isAbortError(error: unknown): boolean {
  return typeof error === 'object' && error !== null && 'name' in error && error.name === 'AbortError'
}

export function isTransientUploadError(error: unknown): boolean {
  if (isAbortError(error)) return false
  if (error instanceof TypeError) return true
  if (typeof error !== 'object' || error === null || !('status' in error)) return false
  return typeof error.status === 'number' && TRANSIENT_HTTP_STATUSES.has(error.status)
}

function validateConcurrency(value: number): number {
  if (!Number.isInteger(value) || value < MIN_UPLOAD_CONCURRENCY || value > MAX_UPLOAD_CONCURRENCY) {
    throw new RangeError(`Upload concurrency must be an integer from ${MIN_UPLOAD_CONCURRENCY} to ${MAX_UPLOAD_CONCURRENCY}`)
  }
  return value
}

function defaultDelay(milliseconds: number, signal: AbortSignal): Promise<void> {
  if (signal.aborted) return Promise.reject(new DOMException('Upload cancelled', 'AbortError'))
  return new Promise((resolve, reject) => {
    const timeout = window.setTimeout(() => {
      signal.removeEventListener('abort', onAbort)
      resolve()
    }, milliseconds)
    const onAbort = () => {
      window.clearTimeout(timeout)
      reject(new DOMException('Upload cancelled', 'AbortError'))
    }
    signal.addEventListener('abort', onAbort, { once: true })
  })
}

export class UploadScheduler<Input, Result> {
  private concurrency: number
  private readonly maxRetries: number
  private readonly baseRetryDelayMs: number
  private readonly maxRetryDelayMs: number
  private readonly jitterRatio: number
  private readonly random: () => number
  private readonly delay: (milliseconds: number, signal: AbortSignal) => Promise<void>
  private readonly isTransientError: (error: unknown) => boolean
  private readonly tasks = new Map<string, ScheduledUpload<Input, Result>>()
  private readonly pendingIds: string[] = []
  private readonly activeIds = new Set<string>()
  private readonly controllers = new Map<string, AbortController>()
  private readonly listeners = new Set<() => void>()
  private snapshot: UploadSchedulerSnapshot<Input, Result>

  constructor(
    private readonly runner: UploadRunner<Input, Result>,
    options: UploadSchedulerOptions = {},
  ) {
    this.concurrency = validateConcurrency(options.concurrency ?? DEFAULT_UPLOAD_CONCURRENCY)
    this.maxRetries = Math.max(0, Math.floor(options.maxRetries ?? 2))
    this.baseRetryDelayMs = Math.max(0, options.baseRetryDelayMs ?? 250)
    this.maxRetryDelayMs = Math.max(this.baseRetryDelayMs, options.maxRetryDelayMs ?? 10_000)
    this.jitterRatio = Math.max(0, options.jitterRatio ?? 0.25)
    this.random = options.random ?? Math.random
    this.delay = options.delay ?? defaultDelay
    this.isTransientError = options.isTransientError ?? isTransientUploadError
    this.snapshot = this.createSnapshot()
  }

  getSnapshot = (): UploadSchedulerSnapshot<Input, Result> => this.snapshot

  getTask(id: string): Readonly<ScheduledUpload<Input, Result>> | undefined {
    const task = this.tasks.get(id)
    return task ? { ...task } : undefined
  }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  enqueue(id: string, input: Input): boolean {
    if (this.tasks.has(id)) return false
    this.tasks.set(id, {
      id,
      input,
      state: 'pending',
      attempts: 0,
      enqueuedAt: Date.now(),
    })
    this.pendingIds.push(id)
    this.publish()
    this.pump()
    return true
  }

  cancel(id: string): boolean {
    const task = this.tasks.get(id)
    if (!task || !['pending', 'active'].includes(task.state)) return false

    task.state = 'cancelled'
    task.completedAt = Date.now()
    if (this.activeIds.has(id)) {
      this.controllers.get(id)?.abort()
    } else {
      const pendingIndex = this.pendingIds.indexOf(id)
      if (pendingIndex >= 0) this.pendingIds.splice(pendingIndex, 1)
    }
    this.publish()
    return true
  }

  setConcurrency(value: number): void {
    const next = validateConcurrency(value)
    if (next === this.concurrency) return
    this.concurrency = next
    this.publish()
    this.pump()
  }

  private pump(): void {
    while (this.activeIds.size < this.concurrency && this.pendingIds.length > 0) {
      const id = this.pendingIds.shift()
      if (!id) return
      const task = this.tasks.get(id)
      if (!task || task.state !== 'pending') continue

      const controller = new AbortController()
      task.state = 'active'
      task.startedAt = Date.now()
      this.activeIds.add(id)
      this.controllers.set(id, controller)
      this.publish()
      void this.run(task, controller)
    }
  }

  private async run(task: ScheduledUpload<Input, Result>, controller: AbortController): Promise<void> {
    while (task.state === 'active') {
      task.attempts += 1
      this.publish()

      try {
        const result = await this.runner(task.input, {
          id: task.id,
          attempt: task.attempts,
          signal: controller.signal,
        })
        if (task.state === 'active') {
          task.result = result
          task.state = 'succeeded'
          task.completedAt = Date.now()
        }
        break
      } catch (error) {
        if (controller.signal.aborted || isAbortError(error)) {
          task.state = 'cancelled'
          task.completedAt ??= Date.now()
          break
        }

        task.error = error
        if (task.attempts > this.maxRetries || !this.isTransientError(error)) {
          task.state = 'failed'
          task.completedAt = Date.now()
          break
        }

        try {
          await this.delay(this.retryDelay(task.attempts), controller.signal)
        } catch (delayError) {
          task.error = delayError
          task.state = controller.signal.aborted || isAbortError(delayError) ? 'cancelled' : 'failed'
          task.completedAt = Date.now()
          break
        }
      }
    }

    this.activeIds.delete(task.id)
    this.controllers.delete(task.id)
    this.publish()
    this.pump()
  }

  private retryDelay(failedAttempt: number): number {
    const exponential = Math.min(
      this.maxRetryDelayMs,
      this.baseRetryDelayMs * 2 ** Math.max(0, failedAttempt - 1),
    )
    return Math.round(exponential * (1 + this.random() * this.jitterRatio))
  }

  private publish(): void {
    this.snapshot = this.createSnapshot()
    for (const listener of [...this.listeners]) listener()
  }

  private createSnapshot(): UploadSchedulerSnapshot<Input, Result> {
    return {
      concurrency: this.concurrency,
      activeCount: this.activeIds.size,
      pendingCount: this.pendingIds.length,
      tasks: [...this.tasks.values()].map((task) => ({ ...task })),
    }
  }
}
