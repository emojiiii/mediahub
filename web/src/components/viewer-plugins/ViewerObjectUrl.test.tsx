import type { PreviewContext, PreviewFile, PreviewPlugin } from '@open-file-viewer/core'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const { createRootSpy, renderSpy, unmountSpy } = vi.hoisted(() => ({
  createRootSpy: vi.fn(),
  renderSpy: vi.fn(),
  unmountSpy: vi.fn(),
}))

vi.mock('react-dom/client', () => ({ createRoot: createRootSpy }))
vi.mock('./ArchivePreviewPlugin', () => ({ default: () => null }))
vi.mock('./SqlitePreviewPlugin', () => ({ default: () => null }))
vi.mock('./SpreadsheetPreviewPlugin', () => ({ default: () => null }))

import { createMediaHubArchivePlugin } from './OpenFileViewerArchivePlugin'
import { createMediaHubSpreadsheetPlugin } from './OpenFileViewerSpreadsheetPlugin'
import { createMediaHubSqlitePlugin } from './OpenFileViewerSqlitePlugin'
import { createViewerObjectUrl } from './ViewerObjectUrl'

const createObjectUrl = vi.fn(() => 'blob:mediahub-preview')
const revokeObjectUrl = vi.fn()
const originalCreateObjectUrl = Object.getOwnPropertyDescriptor(URL, 'createObjectURL')
const originalRevokeObjectUrl = Object.getOwnPropertyDescriptor(URL, 'revokeObjectURL')

beforeEach(() => {
  createObjectUrl.mockClear()
  revokeObjectUrl.mockClear()
  renderSpy.mockClear()
  unmountSpy.mockClear()
  createRootSpy.mockReset().mockReturnValue({ render: renderSpy, unmount: unmountSpy })
  Object.defineProperty(URL, 'createObjectURL', { configurable: true, value: createObjectUrl })
  Object.defineProperty(URL, 'revokeObjectURL', { configurable: true, value: revokeObjectUrl })
})

afterEach(() => {
  restoreUrlMethod('createObjectURL', originalCreateObjectUrl)
  restoreUrlMethod('revokeObjectURL', originalRevokeObjectUrl)
})

describe('createViewerObjectUrl', () => {
  it('creates one revocable URL for a File and makes revoke idempotent', () => {
    const file = new File([new Uint8Array([1])], 'data.db')
    const objectUrl = createViewerObjectUrl(previewFile(file, 'data.db', 'db', 'application/vnd.sqlite3'))

    expect(objectUrl.url).toBe('blob:mediahub-preview')
    expect(createObjectUrl).toHaveBeenCalledOnce()
    objectUrl.revoke()
    objectUrl.revoke()
    expect(revokeObjectUrl).toHaveBeenCalledOnce()
    expect(revokeObjectUrl).toHaveBeenCalledWith('blob:mediahub-preview')
  })

  it('keeps a remote URL external and never revokes it', () => {
    const objectUrl = createViewerObjectUrl({
      source: '/objects/data.db',
      url: '/objects/data.db',
      name: 'data.db',
      extension: 'db',
      mimeType: 'application/vnd.sqlite3',
    })
    objectUrl.revoke()
    expect(objectUrl.url).toBe('/objects/data.db')
    expect(createObjectUrl).not.toHaveBeenCalled()
    expect(revokeObjectUrl).not.toHaveBeenCalled()
  })
})

describe('custom buffered preview adapters', () => {
  it.each([
    ['archive', () => createMediaHubArchivePlugin(1), 'release.zip', 'zip', 'application/zip'],
    ['sqlite', () => createMediaHubSqlitePlugin(1), 'data.sqlite', 'sqlite', 'application/vnd.sqlite3'],
    ['spreadsheet', () => createMediaHubSpreadsheetPlugin(1), 'sheet.xlsx', 'xlsx', 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'],
  ] as const)('uses and revokes a local Blob URL in the %s adapter', async (_name, createPlugin, fileName, extension, mimeType) => {
    const file = new File([new Uint8Array([1])], fileName, { type: mimeType })
    const viewport = document.createElement('div')
    const instance = await (createPlugin as () => PreviewPlugin)().render({
      file: previewFile(file, fileName, extension, mimeType),
      viewport,
    } as unknown as PreviewContext)

    expect(createObjectUrl).toHaveBeenCalledWith(file)
    expect(renderSpy).toHaveBeenCalledOnce()
    expect(renderSpy.mock.calls[0][0]).toMatchObject({ props: { url: 'blob:mediahub-preview', size: 1 } })
    expect(viewport.firstElementChild).not.toBeNull()
    instance.destroy()
    await new Promise<void>((resolve) => queueMicrotask(resolve))
    expect(unmountSpy).toHaveBeenCalledOnce()
    expect(viewport.firstElementChild).toBeNull()
    expect(revokeObjectUrl).toHaveBeenCalledOnce()
    expect(revokeObjectUrl).toHaveBeenCalledWith('blob:mediahub-preview')
  })
})

function previewFile(file: File, name: string, extension = 'db', mimeType = ''): PreviewFile {
  return { source: file, blob: file, size: file.size, name, extension, mimeType }
}

function restoreUrlMethod(name: 'createObjectURL' | 'revokeObjectURL', descriptor: PropertyDescriptor | undefined) {
  if (descriptor) Object.defineProperty(URL, name, descriptor)
  else Reflect.deleteProperty(URL, name)
}
