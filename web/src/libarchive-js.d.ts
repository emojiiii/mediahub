declare module 'libarchive.js/src/webworker/archive-reader.js' {
  export type LibArchiveReaderEntry = {
    size: number
    path: string
    type: string
    lastModified: number
    fileName?: string
    ref: number
    fileData?: Uint8Array
  }

  export class ArchiveReader {
    constructor(wasmModule: unknown)
    open(file: Blob): Promise<void>
    close(): void
    setPassphrase(passphrase: string): void
    entries(skipExtraction?: boolean, except?: string | null): Generator<LibArchiveReaderEntry>
  }
}

declare module 'libarchive.js/src/webworker/wasm-module.js' {
  export class WasmModule {
    locateFile?: (path: string, prefix: string) => string
  }

  export function getWasmModule(callback: (module: unknown) => void): void
}
