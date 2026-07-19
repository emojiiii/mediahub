import { describe, expect, it, vi } from 'vitest'

import { isTransientUploadError, UploadScheduler } from './upload-scheduler'

type Deferred<T> = {
  promise: Promise<T>
  resolve: (value: T) => void
  reject: (error: unknown) => void
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}

describe('UploadScheduler', () => {
  it('uses a default concurrency of four and never exceeds it', async () => {
    const releases = new Map(
      Array.from({ length: 6 }, (_, index) => {
        const id = String(index + 1)
        return [id, deferred<string>()] as const
      }),
    )
    let running = 0
    let maximumRunning = 0
    const scheduler = new UploadScheduler<string, string>(async (input) => {
      running += 1
      maximumRunning = Math.max(maximumRunning, running)
      const result = await releases.get(input)?.promise
      running -= 1
      return result ?? input
    })

    for (let index = 1; index <= 6; index += 1) scheduler.enqueue(String(index), String(index))
    expect(scheduler.getSnapshot()).toMatchObject({ concurrency: 4, activeCount: 4, pendingCount: 2 })

    releases.get('1')?.resolve('1')
    await vi.waitFor(() => expect(scheduler.getTask('5')?.state).toBe('active'))
    expect(maximumRunning).toBe(4)

    for (const [id, release] of releases) release.resolve(id)
    await vi.waitFor(() => expect(scheduler.getSnapshot().activeCount).toBe(0))
    expect(maximumRunning).toBe(4)
  })

  it('starts pending tasks in FIFO order and applies concurrency changes', async () => {
    const started: string[] = []
    const releases = new Map<string, Deferred<string>>()
    const scheduler = new UploadScheduler<string, string>(async (input) => {
      started.push(input)
      const release = deferred<string>()
      releases.set(input, release)
      return release.promise
    }, { concurrency: 1 })

    scheduler.enqueue('first', 'first')
    scheduler.enqueue('second', 'second')
    scheduler.enqueue('third', 'third')
    expect(started).toEqual(['first'])

    scheduler.setConcurrency(2)
    expect(started).toEqual(['first', 'second'])
    releases.get('second')?.resolve('second')
    await vi.waitFor(() => expect(started).toEqual(['first', 'second', 'third']))

    releases.get('first')?.resolve('first')
    releases.get('third')?.resolve('third')
    await vi.waitFor(() => expect(scheduler.getSnapshot().activeCount).toBe(0))
    expect(() => scheduler.setConcurrency(13)).toThrow(RangeError)
  })

  it('does not start more queued work until a reduced concurrency is respected', async () => {
    const started: string[] = []
    const releases = new Map(
      Array.from({ length: 5 }, (_, index) => {
        const id = String(index + 1)
        return [id, deferred<string>()] as const
      }),
    )
    const scheduler = new UploadScheduler<string, string>(async (input) => {
      started.push(input)
      return releases.get(input)?.promise ?? input
    }, { concurrency: 3 })

    for (let index = 1; index <= 5; index += 1) scheduler.enqueue(String(index), String(index))
    expect(started).toEqual(['1', '2', '3'])

    scheduler.setConcurrency(1)
    releases.get('1')?.resolve('1')
    await vi.waitFor(() => expect(scheduler.getTask('1')?.state).toBe('succeeded'))
    releases.get('2')?.resolve('2')
    await vi.waitFor(() => expect(scheduler.getTask('2')?.state).toBe('succeeded'))
    expect(started).toEqual(['1', '2', '3'])

    releases.get('3')?.resolve('3')
    await vi.waitFor(() => expect(started).toEqual(['1', '2', '3', '4']))
    releases.get('4')?.resolve('4')
    await vi.waitFor(() => expect(started).toEqual(['1', '2', '3', '4', '5']))
    releases.get('5')?.resolve('5')
    await vi.waitFor(() => expect(scheduler.getSnapshot().activeCount).toBe(0))
  })

  it('continues with the next task after a non-retryable failure', async () => {
    const started: string[] = []
    const scheduler = new UploadScheduler<string, string>(async (input) => {
      started.push(input)
      if (input === 'bad') throw Object.assign(new Error('invalid'), { status: 400 })
      return input
    }, { concurrency: 1 })

    scheduler.enqueue('bad', 'bad')
    scheduler.enqueue('good', 'good')

    await vi.waitFor(() => expect(scheduler.getTask('good')?.state).toBe('succeeded'))
    expect(started).toEqual(['bad', 'good'])
    expect(scheduler.getTask('bad')).toMatchObject({ state: 'failed', attempts: 1 })
  })

  it('cancels a pending task and rejects duplicate task IDs', async () => {
    const firstRelease = deferred<string>()
    const started: string[] = []
    const scheduler = new UploadScheduler<string, string>(async (input) => {
      started.push(input)
      return input === 'first' ? firstRelease.promise : input
    }, { concurrency: 1 })

    expect(scheduler.enqueue('first', 'first')).toBe(true)
    expect(scheduler.enqueue('second', 'second')).toBe(true)
    expect(scheduler.enqueue('second', 'duplicate')).toBe(false)
    expect(scheduler.cancel('second')).toBe(true)
    expect(scheduler.getTask('second')?.state).toBe('cancelled')

    firstRelease.resolve('first')
    await vi.waitFor(() => expect(scheduler.getSnapshot().activeCount).toBe(0))
    expect(started).toEqual(['first'])
  })

  it('aborts an active task and releases its slot to the next queued task', async () => {
    let activeSignal: AbortSignal | undefined
    const scheduler = new UploadScheduler<string, string>((input, context) => {
      if (input === 'next') return Promise.resolve(input)
      activeSignal = context.signal
      return new Promise((_resolve, reject) => {
        context.signal.addEventListener('abort', () => {
          reject(new DOMException('Upload cancelled', 'AbortError'))
        }, { once: true })
      })
    }, { concurrency: 1 })

    scheduler.enqueue('active', 'active')
    scheduler.enqueue('next', 'next')
    expect(scheduler.cancel('active')).toBe(true)

    await vi.waitFor(() => expect(scheduler.getTask('next')?.state).toBe('succeeded'))
    expect(activeSignal?.aborted).toBe(true)
    expect(scheduler.getTask('active')).toMatchObject({ state: 'cancelled', attempts: 1 })
    expect(scheduler.cancel('active')).toBe(false)
  })

  it('retries transient failures twice with exponential backoff', async () => {
    vi.useFakeTimers()
    const runner = vi.fn(async () => {
      if (runner.mock.calls.length < 3) throw Object.assign(new Error('busy'), { status: 503 })
      return 'uploaded'
    })
    const scheduler = new UploadScheduler<string, string>(runner, {
      concurrency: 1,
      baseRetryDelayMs: 100,
      jitterRatio: 0,
    })

    scheduler.enqueue('retry-me', 'retry-me')
    expect(runner).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(100)
    expect(runner).toHaveBeenCalledTimes(2)
    await vi.advanceTimersByTimeAsync(200)
    expect(runner).toHaveBeenCalledTimes(3)
    expect(scheduler.getTask('retry-me')).toMatchObject({ state: 'succeeded', attempts: 3 })
    vi.useRealTimers()
  })

  it('stops retrying when an upload is cancelled during backoff', async () => {
    const delayStarted = deferred<void>()
    const runner = vi.fn(async () => {
      throw Object.assign(new Error('busy'), { status: 503 })
    })
    const delay = vi.fn((_milliseconds: number, signal: AbortSignal) => {
      delayStarted.resolve()
      return new Promise<void>((_resolve, reject) => {
        signal.addEventListener('abort', () => {
          reject(new DOMException('Upload cancelled', 'AbortError'))
        }, { once: true })
      })
    })
    const scheduler = new UploadScheduler<string, string>(runner, {
      concurrency: 1,
      delay,
    })

    scheduler.enqueue('retrying', 'retrying')
    await delayStarted.promise
    expect(scheduler.cancel('retrying')).toBe(true)

    await vi.waitFor(() => expect(scheduler.getSnapshot().activeCount).toBe(0))
    expect(runner).toHaveBeenCalledTimes(1)
    expect(delay).toHaveBeenCalledTimes(1)
    expect(scheduler.getTask('retrying')).toMatchObject({ state: 'cancelled', attempts: 1 })
  })

  it('recognizes retryable HTTP and network failures but never AbortError', () => {
    expect(isTransientUploadError(Object.assign(new Error('rate limited'), { status: 429 }))).toBe(true)
    expect(isTransientUploadError(Object.assign(new Error('gateway'), { status: 502 }))).toBe(true)
    expect(isTransientUploadError(new TypeError('Failed to fetch'))).toBe(true)
    expect(isTransientUploadError(new DOMException('cancelled', 'AbortError'))).toBe(false)
    expect(isTransientUploadError(Object.assign(new Error('bad request'), { status: 400 }))).toBe(false)
  })
})
