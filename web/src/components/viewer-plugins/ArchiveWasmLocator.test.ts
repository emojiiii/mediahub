import { afterEach, describe, expect, it } from 'vitest'

import { WasmModule } from 'libarchive.js/src/webworker/wasm-module.js'

import { configureArchiveWasmLocation, resolveArchiveWasmFile } from './ArchiveWasmLocator'

afterEach(() => {
  delete WasmModule.prototype.locateFile
})

describe('ArchiveWasmLocator', () => {
  it('replaces the pre-bundled dependency-relative WASM path with the Vite asset URL', () => {
    expect(resolveArchiveWasmFile('libarchive.wasm', '/node_modules/.vite/deps/', '/assets/libarchive-hash.wasm'))
      .toBe('/assets/libarchive-hash.wasm')
  })

  it('keeps the normal prefix behavior for unrelated runtime files', () => {
    expect(resolveArchiveWasmFile('support.data', '/worker/', '/assets/libarchive-hash.wasm'))
      .toBe('/worker/support.data')
  })

  it('installs locateFile on every libarchive module instance', () => {
    configureArchiveWasmLocation('/assets/libarchive-hash.wasm')

    expect(new WasmModule().locateFile?.('libarchive.wasm', '/node_modules/.vite/deps/'))
      .toBe('/assets/libarchive-hash.wasm')
  })
})
