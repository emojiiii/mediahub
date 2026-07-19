declare module 'sevenzip-wasm' {
  type SevenZipOptions = {
    locateFile?: (path: string, prefix: string) => string
    print?: (line: string) => void
    printErr?: (line: string) => void
  }

  type SevenZipModule = {
    FS: unknown
    callMain(args: string[]): number
  }

  const SevenZipWasm: (options?: SevenZipOptions) => Promise<SevenZipModule>
  export default SevenZipWasm
}

declare module 'sevenzip-wasm/sevenzip-wasm.wasm?url' {
  const url: string
  export default url
}
