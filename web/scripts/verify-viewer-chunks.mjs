import { readdir, readFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const webRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const distRoot = path.join(webRoot, 'dist')
const assetsRoot = path.join(distRoot, 'assets')
const assetNames = await readdir(assetsRoot)

function findAsset(pattern, label) {
  const matches = assetNames.filter((name) => pattern.test(name))
  if (matches.length !== 1) throw new Error(`Expected one ${label} asset, found ${matches.length}: ${matches.join(', ')}`)
  return matches[0]
}

function requireReference(content, asset, owner) {
  if (!content.includes(asset)) throw new Error(`${owner} does not reference ${asset}`)
}

function forbidReference(content, asset, owner) {
  if (content.includes(asset)) throw new Error(`${owner} eagerly references ${asset}`)
}

const html = await readFile(path.join(distRoot, 'index.html'), 'utf8')
const mainMatch = html.match(/<script[^>]+src="\/assets\/([^"?]+\.js)"/)
if (!mainMatch) throw new Error('Unable to resolve the main JavaScript asset from index.html')

const mainAsset = mainMatch[1]
const objectViewerAsset = findAsset(/^ObjectFileViewer-.*\.js$/, 'ObjectFileViewer JavaScript')
const objectViewerCssAsset = findAsset(/^ObjectFileViewer-.*\.css$/, 'ObjectFileViewer CSS')
const archivePluginAsset = findAsset(/^ArchivePreviewPlugin-.*\.js$/, 'ArchivePreviewPlugin')
const archiveWorkerAsset = findAsset(/^archive\.worker-.*\.js$/, 'archive Worker')
const archiveWasmAsset = findAsset(/^libarchive-.*\.wasm$/, 'libarchive WASM')
const sevenZipAsset = findAsset(/^SevenZipArchive-.*\.js$/, '7-Zip password adapter')
const sevenZipWasmAsset = findAsset(/^sevenzip-wasm-.*\.wasm$/, '7-Zip password WASM')
const sqlitePluginAsset = findAsset(/^SqlitePreviewPlugin-.*\.js$/, 'SQLite preview plugin')
const sqliteWorkerAsset = findAsset(/^sqlite\.worker-.*\.js$/, 'SQLite Worker')
const sqliteWasmAsset = findAsset(/^sql-wasm-.*\.wasm$/, 'sql.js WASM')
const spreadsheetPluginAsset = findAsset(/^SpreadsheetPreviewPlugin-.*\.js$/, 'spreadsheet preview plugin')
const spreadsheetWorkerAsset = findAsset(/^spreadsheet\.worker-.*\.js$/, 'spreadsheet Worker')
const pdfWorkerAsset = findAsset(/^pdf\.worker-.*\.mjs$/, 'PDF.js Worker')
const heavyFormatAssets = [
  findAsset(/^xlsx-.*\.js$/, 'SheetJS parser'),
  findAsset(/^pdf-.*\.js$/, 'PDF.js parser'),
  findAsset(/^three\.module-.*\.js$/, 'Three.js runtime'),
  findAsset(/^aiden0z-pptx-renderer\.es-.*\.js$/, 'PPTX renderer'),
  findAsset(/^heic2any-.*\.js$/, 'HEIC converter'),
  findAsset(/^docx-preview-.*\.js$/, 'DOCX renderer'),
  findAsset(/^prism-typescript-.*\.js$/, 'TypeScript grammar'),
]

const [main, objectViewer, archivePlugin, archiveWorker, sevenZip, sqlitePlugin, sqliteWorker, spreadsheetPlugin, spreadsheetWorker] = await Promise.all([
  readFile(path.join(assetsRoot, mainAsset), 'utf8'),
  readFile(path.join(assetsRoot, objectViewerAsset), 'utf8'),
  readFile(path.join(assetsRoot, archivePluginAsset), 'utf8'),
  readFile(path.join(assetsRoot, archiveWorkerAsset), 'utf8'),
  readFile(path.join(assetsRoot, sevenZipAsset), 'utf8'),
  readFile(path.join(assetsRoot, sqlitePluginAsset), 'utf8'),
  readFile(path.join(assetsRoot, sqliteWorkerAsset), 'utf8'),
  readFile(path.join(assetsRoot, spreadsheetPluginAsset), 'utf8'),
  readFile(path.join(assetsRoot, spreadsheetWorkerAsset), 'utf8'),
])

requireReference(main, objectViewerAsset, mainAsset)
forbidReference(html, objectViewerAsset, 'index.html')
forbidReference(html, objectViewerCssAsset, 'index.html')

const deepViewerAssets = [
  archivePluginAsset,
  archiveWorkerAsset,
  archiveWasmAsset,
  sevenZipAsset,
  sevenZipWasmAsset,
  sqlitePluginAsset,
  sqliteWorkerAsset,
  sqliteWasmAsset,
  spreadsheetPluginAsset,
  spreadsheetWorkerAsset,
  pdfWorkerAsset,
  ...heavyFormatAssets,
]
for (const asset of deepViewerAssets) {
  forbidReference(html, asset, 'index.html')
  forbidReference(main, asset, mainAsset)
}

requireReference(objectViewer, archivePluginAsset, objectViewerAsset)
requireReference(objectViewer, pdfWorkerAsset, objectViewerAsset)
for (const asset of heavyFormatAssets) requireReference(objectViewer, asset, objectViewerAsset)
forbidReference(objectViewer, archiveWorkerAsset, objectViewerAsset)
forbidReference(objectViewer, archiveWasmAsset, objectViewerAsset)
forbidReference(objectViewer, sevenZipAsset, objectViewerAsset)
forbidReference(objectViewer, sevenZipWasmAsset, objectViewerAsset)
requireReference(archivePlugin, archiveWorkerAsset, archivePluginAsset)
forbidReference(archivePlugin, archiveWasmAsset, archivePluginAsset)
forbidReference(archivePlugin, sevenZipAsset, archivePluginAsset)
forbidReference(archivePlugin, sevenZipWasmAsset, archivePluginAsset)
requireReference(archiveWorker, archiveWasmAsset, archiveWorkerAsset)
requireReference(archiveWorker, sevenZipAsset, archiveWorkerAsset)
forbidReference(archiveWorker, sevenZipWasmAsset, archiveWorkerAsset)
requireReference(sevenZip, sevenZipWasmAsset, sevenZipAsset)

requireReference(objectViewer, sqlitePluginAsset, objectViewerAsset)
forbidReference(objectViewer, sqliteWorkerAsset, objectViewerAsset)
forbidReference(objectViewer, sqliteWasmAsset, objectViewerAsset)
requireReference(sqlitePlugin, sqliteWorkerAsset, sqlitePluginAsset)
forbidReference(sqlitePlugin, sqliteWasmAsset, sqlitePluginAsset)
requireReference(sqliteWorker, sqliteWasmAsset, sqliteWorkerAsset)

requireReference(objectViewer, spreadsheetPluginAsset, objectViewerAsset)
forbidReference(objectViewer, spreadsheetWorkerAsset, objectViewerAsset)
requireReference(spreadsheetPlugin, spreadsheetWorkerAsset, spreadsheetPluginAsset)
if (!spreadsheetWorker.includes('SheetJS')) throw new Error(`${spreadsheetWorkerAsset} does not contain the SheetJS parser`)

const legacyPatterns = [
  /^SourcePreviewPlugin-/,
  /^FontPreviewPlugin-/,
]
for (const pattern of legacyPatterns) {
  const matches = assetNames.filter((name) => pattern.test(name))
  if (matches.length > 0) throw new Error(`Superseded local viewer assets were emitted: ${matches.join(', ')}`)
}

const [cMapFiles, standardFontFiles] = await Promise.all([
  readdir(path.join(distRoot, 'pdfjs', 'cmaps')),
  readdir(path.join(distRoot, 'pdfjs', 'standard_fonts')),
])
if (cMapFiles.length < 100) throw new Error(`Expected local PDF CMaps, found only ${cMapFiles.length}`)
if (standardFontFiles.length < 10) throw new Error(`Expected local PDF standard fonts, found only ${standardFontFiles.length}`)
if (!objectViewer.includes('pdfjs/') || !objectViewer.includes('cmaps/') || !objectViewer.includes('standard_fonts/')) {
  throw new Error('ObjectFileViewer does not reference the locally bundled PDF support assets')
}

console.log(
  `Lazy viewer chunks verified: ${mainAsset} -> ${objectViewerAsset} -> upstream format chunks; `
  + `${archivePluginAsset} -> ${archiveWorkerAsset} -> ${archiveWasmAsset}; password only -> ${sevenZipAsset} -> ${sevenZipWasmAsset}; `
  + `${sqlitePluginAsset} -> ${sqliteWorkerAsset} -> ${sqliteWasmAsset}; `
  + `${spreadsheetPluginAsset} -> ${spreadsheetWorkerAsset} (SheetJS); `
  + `${cMapFiles.length} CMaps and ${standardFontFiles.length} PDF fonts are local`,
)
