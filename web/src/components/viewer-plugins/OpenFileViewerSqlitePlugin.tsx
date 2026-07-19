import type { PreviewFile, PreviewPlugin } from '@open-file-viewer/core'
import { createRoot } from 'react-dom/client'

import { deferViewerRootCleanup } from './ViewerShared'
import { createViewerObjectUrl } from './ViewerObjectUrl'

const sqliteExtensions = new Set(['db', 'db3', 'sdb', 'sqlite', 'sqlite3'])
const sqliteMimeTypes = new Set([
  'application/sqlite3',
  'application/vnd.sqlite3',
  'application/x-sqlite',
  'application/x-sqlite3',
])

export function isSqliteFile(file: Pick<PreviewFile, 'extension' | 'mimeType'>): boolean {
  const extension = file.extension.toLowerCase().replace(/^\./, '')
  const mimeType = file.mimeType.toLowerCase().split(';', 1)[0].trim()
  return sqliteExtensions.has(extension) || sqliteMimeTypes.has(mimeType)
}

export function createMediaHubSqlitePlugin(sourceSize: number): PreviewPlugin {
  return {
    name: 'mediahub-sqlite',
    match: isSqliteFile,
    async render(ctx) {
      const { default: SqlitePreviewPlugin } = await import('./SqlitePreviewPlugin')
      const objectUrl = createViewerObjectUrl(ctx.file)
      const mount = document.createElement('div')
      mount.className = 'h-full min-h-0 w-full min-w-0 overflow-hidden'
      ctx.viewport.append(mount)
      const root = createRoot(mount)
      root.render(<SqlitePreviewPlugin
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
