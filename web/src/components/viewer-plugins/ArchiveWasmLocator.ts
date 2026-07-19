import archiveWasmUrl from 'libarchive.js/src/webworker/wasm-gen/libarchive.wasm?url'
import { WasmModule } from 'libarchive.js/src/webworker/wasm-module.js'

export function resolveArchiveWasmFile(path: string, prefix: string, wasmUrl = archiveWasmUrl): string {
  return path === 'libarchive.wasm' ? wasmUrl : `${prefix}${path}`
}

export function configureArchiveWasmLocation(wasmUrl = archiveWasmUrl): void {
  WasmModule.prototype.locateFile = (path, prefix) => resolveArchiveWasmFile(path, prefix, wasmUrl)
}
