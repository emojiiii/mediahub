import SevenZipWasm from 'sevenzip-wasm'
import sevenZipWasmUrl from 'sevenzip-wasm/sevenzip-wasm.wasm?url'

import type { ArchiveEntrySummary } from './archive-protocol'
import { normalizeArchiveEntryPath } from './ArchivePreviewPolicy'

const ARCHIVE_PATH = '/mediahub-archive'
const OUTPUT_PATH = '/mediahub-output'
const MAX_CAPTURED_OUTPUT_LINES = 50_000

type SevenZipFileSystem = {
  writeFile(path: string, data: Uint8Array): void
  readFile(path: string): Uint8Array
  mkdir(path: string): void
  stat(path: string): { mode: number; size: number }
  lstat(path: string): { mode: number; size: number }
  isFile(mode: number): boolean
  isLink(mode: number): boolean
}

type SevenZipModule = {
  FS: SevenZipFileSystem
  callMain(args: string[]): number
}

type SevenZipSession = {
  module: SevenZipModule
  output: string[]
  errors: string[]
  outputTruncated: boolean
}

export type SevenZipListResult = {
  entries: ArchiveEntrySummary[]
  encrypted: boolean
}

export type SevenZipExtractedFile = {
  path: string
  data: Uint8Array
}

export class SevenZipPasswordError extends Error {
  constructor() {
    super('Incorrect archive password')
    this.name = 'SevenZipPasswordError'
  }
}

export async function inspectWithSevenZip(file: File, password: string): Promise<SevenZipListResult> {
  const session = await createSession(file)
  runSevenZip(session, ['t', '-bd', '-bb0', '-y', passwordSwitch(password), ARCHIVE_PATH])
  const output = runSevenZip(session, ['l', '-slt', '-bd', '-y', passwordSwitch(password), ARCHIVE_PATH])
  return parseSevenZipListOutput(output)
}

export async function extractWithSevenZip(
  file: File,
  password: string,
  selected: ArchiveEntrySummary[],
): Promise<SevenZipExtractedFile[]> {
  const session = await createSession(file)
  session.module.FS.mkdir(OUTPUT_PATH)
  runSevenZip(session, [
    'x',
    '-bd',
    '-bb0',
    '-y',
    '-spd',
    passwordSwitch(password),
    `-o${OUTPUT_PATH}`,
    ARCHIVE_PATH,
    '--',
    ...selected.map((entry) => entry.path),
  ])

  return selected.map((entry) => {
    const outputPath = `${OUTPUT_PATH}/${entry.path}`
    const stat = session.module.FS.stat(outputPath)
    const linkStat = session.module.FS.lstat(outputPath)
    if (!session.module.FS.isFile(stat.mode) || session.module.FS.isLink(linkStat.mode)) {
      throw new Error(`7-Zip did not extract a regular file: ${entry.path}`)
    }
    const data = session.module.FS.readFile(outputPath).slice()
    if (stat.size !== entry.size || data.byteLength !== entry.size) {
      throw new Error(`7-Zip extracted an unexpected file size: ${entry.path}`)
    }
    return { path: entry.path, data }
  })
}

export function parseSevenZipListOutput(lines: string[]): SevenZipListResult {
  const normalizedLines = lines.flatMap((line) => line.replace(/\r/g, '').split('\n'))
  const separatorIndex = normalizedLines.findIndex((line) => line.trim() === '----------')
  if (separatorIndex < 0) throw new Error('7-Zip listing did not contain an entry section')

  const blocks: Array<Map<string, string>> = []
  let fields = new Map<string, string>()
  const finishBlock = () => {
    if (fields.size > 0) blocks.push(fields)
    fields = new Map<string, string>()
  }

  for (const line of normalizedLines.slice(separatorIndex + 1)) {
    if (line.length === 0) {
      finishBlock()
      continue
    }
    const delimiterIndex = line.indexOf(' = ')
    if (delimiterIndex <= 0) continue
    fields.set(line.slice(0, delimiterIndex), line.slice(delimiterIndex + 3))
  }
  finishBlock()

  let encrypted = false
  const entries = blocks.map((block): ArchiveEntrySummary | null => {
    const rawPath = block.get('Path')
    if (!rawPath) return null
    const path = normalizeArchiveEntryPath(rawPath)
    if (!path) return null

    const attributes = block.get('Attributes') || ''
    if (block.has('Symbolic Link') || block.has('Hard Link') || /(^|[ _-])l[rwx-]/i.test(attributes)) {
      throw new Error(`7-Zip archive contains a link entry: ${path}`)
    }
    const directory = block.get('Folder') === '+' || attributes.startsWith('D')
    const rawSize = block.get('Size') || '0'
    if (!/^\d+$/.test(rawSize)) throw new Error(`7-Zip reported an invalid entry size: ${path}`)
    const size = directory ? 0 : Number(rawSize)
    if (!Number.isSafeInteger(size)) throw new Error(`7-Zip reported an unsafe entry size: ${path}`)
    encrypted = encrypted || block.get('Encrypted') === '+'
    return { path, size, directory }
  }).filter((entry): entry is ArchiveEntrySummary => entry !== null)

  return { entries, encrypted }
}

async function createSession(file: File): Promise<SevenZipSession> {
  const output: string[] = []
  const errors: string[] = []
  const session: SevenZipSession = {
    module: undefined as unknown as SevenZipModule,
    output,
    errors,
    outputTruncated: false,
  }
  const capture = (target: string[]) => (line: string) => {
    if (target.length >= MAX_CAPTURED_OUTPUT_LINES) {
      session.outputTruncated = true
      return
    }
    target.push(line)
  }
  session.module = await SevenZipWasm({
    locateFile: (path: string) => path.endsWith('.wasm') ? sevenZipWasmUrl : path,
    print: capture(output),
    printErr: capture(errors),
  }) as SevenZipModule
  session.module.FS.writeFile(ARCHIVE_PATH, new Uint8Array(await file.arrayBuffer()))
  return session
}

function runSevenZip(session: SevenZipSession, args: string[]): string[] {
  session.output.length = 0
  session.errors.length = 0
  session.outputTruncated = false
  const exitCode = session.module.callMain(args)
  if (session.outputTruncated) throw new Error('7-Zip output exceeded the safety limit')
  if (exitCode === 0) return [...session.output]

  const detail = [...session.errors, ...session.output].join('\n')
  if (/wrong password|password is not defined|encrypted archive|crc failed in encrypted file/i.test(detail)) {
    throw new SevenZipPasswordError()
  }
  const conciseDetail = detail.replace(/\r/g, '').split('\n').map((line) => line.trim()).filter(Boolean).slice(-4).join(' ')
  throw new Error(conciseDetail ? `7-Zip failed: ${conciseDetail}` : `7-Zip exited with code ${exitCode}`)
}

function passwordSwitch(password: string): string {
  return `-p${password}`
}
