import type { PreviewFile, PreviewPlugin } from '@open-file-viewer/core'
import { createRoot } from 'react-dom/client'

import { deferViewerRootCleanup } from './ViewerShared'
import { createViewerObjectUrl } from './ViewerObjectUrl'

const archiveExtensions = new Set([
  '7z', 'bz2', 'bzip2', 'gz', 'gzip', 'lzma', 'rar', 'tar', 'tbz', 'tbz2', 'tgz', 'txz', 'xz', 'zip',
])
const archiveMimeTypes = new Set([
  'application/gzip',
  'application/vnd.rar',
  'application/x-7z-compressed',
  'application/x-bzip2',
  'application/x-compressed-tar',
  'application/x-gzip',
  'application/x-gtar',
  'application/x-lzma',
  'application/x-rar-compressed',
  'application/x-tar',
  'application/x-xz',
  'application/x-zip-compressed',
  'application/zip',
])

export function isArchiveFile(file: Pick<PreviewFile, 'extension' | 'mimeType'>): boolean {
  return archiveExtensions.has(file.extension.toLowerCase()) || archiveMimeTypes.has(file.mimeType.toLowerCase())
}

export function createMediaHubArchivePlugin(sourceSize: number): PreviewPlugin {
  return {
    name: 'mediahub-archive',
    match: isArchiveFile,
    async render(ctx) {
      const [{ default: ArchivePreviewPlugin }] = await Promise.all([
        import('./ArchivePreviewPlugin'),
      ])
      const objectUrl = createViewerObjectUrl(ctx.file)
      const mount = document.createElement('div')
      mount.className = 'h-full min-h-0 w-full min-w-0 overflow-hidden'
      ctx.viewport.append(mount)
      const root = createRoot(mount)
      root.render(<ArchivePreviewPlugin
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
