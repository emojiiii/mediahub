import type { PreviewFile, PreviewPlugin } from '@open-file-viewer/core'
import { createRoot } from 'react-dom/client'

import { deferViewerRootCleanup } from './ViewerShared'
import { createViewerObjectUrl } from './ViewerObjectUrl'

const spreadsheetExtensions = new Set([
  'csv', 'ods', 'tsv', 'xls', 'xlsb', 'xlsm', 'xlsx',
])
const spreadsheetMimeTypes = new Set([
  'application/csv',
  'application/vnd.ms-excel',
  'application/vnd.ms-excel.sheet.binary.macroenabled.12',
  'application/vnd.ms-excel.sheet.macroenabled.12',
  'application/vnd.oasis.opendocument.spreadsheet',
  'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
  'text/csv',
  'text/tab-separated-values',
])

export function isSpreadsheetFile(file: Pick<PreviewFile, 'extension' | 'mimeType'>): boolean {
  const extension = file.extension.toLowerCase().replace(/^\./, '')
  const mimeType = file.mimeType.toLowerCase().split(';', 1)[0].trim()
  return spreadsheetExtensions.has(extension) || spreadsheetMimeTypes.has(mimeType)
}

export function createMediaHubSpreadsheetPlugin(sourceSize: number): PreviewPlugin {
  return {
    name: 'mediahub-spreadsheet',
    match: isSpreadsheetFile,
    async render(ctx) {
      const { default: SpreadsheetPreviewPlugin } = await import('./SpreadsheetPreviewPlugin')
      const objectUrl = createViewerObjectUrl(ctx.file)
      const mount = document.createElement('div')
      mount.className = 'h-full min-h-0 w-full min-w-0 overflow-hidden'
      ctx.viewport.append(mount)
      const root = createRoot(mount)
      root.render(<SpreadsheetPreviewPlugin
        fileName={ctx.file.name}
        mimeType={ctx.file.mimeType}
        size={sourceSize}
        url={objectUrl.url}
      />)

      return {
        destroy() {
          deferViewerRootCleanup(root, mount, objectUrl.revoke)
        },
      }
    },
  }
}
