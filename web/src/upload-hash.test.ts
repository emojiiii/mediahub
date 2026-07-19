import { describe, expect, it } from 'vitest'

import { sha256File } from './upload-hash'

describe('sha256File', () => {
  it('hashes a file incrementally across bounded slices', async () => {
    const file = new File(['abc', 'def'], 'payload.bin')
    const slice = file.slice.bind(file)
    const ranges: Array<[number | undefined, number | undefined]> = []
    file.slice = (start, end, contentType) => {
      ranges.push([start, end])
      return slice(start, end, contentType)
    }

    await expect(sha256File(file, undefined, 3)).resolves.toBe('bef57ec7f53a6d40beb640a780a639c83bc29ac8a9816f1fc6c5c6dcd93c4721')
    expect(ranges).toEqual([[0, 3], [3, 6]])
  })

  it('stops before reading when the upload is cancelled', async () => {
    const controller = new AbortController()
    controller.abort(new DOMException('上传已取消', 'AbortError'))

    await expect(sha256File(new File(['content'], 'cancelled.bin'), controller.signal)).rejects.toMatchObject({ name: 'AbortError' })
  })
})
