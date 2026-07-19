import type { PreviewFile } from '@open-file-viewer/core'

export type ViewerObjectUrl = {
  url: string
  revoke(): void
}

export function createViewerObjectUrl(file: PreviewFile): ViewerObjectUrl {
  const blob = file.blob ?? (file.source instanceof Blob ? file.source : null)
  if (blob) {
    const url = URL.createObjectURL(blob)
    let revoked = false
    return {
      url,
      revoke() {
        if (revoked) return
        revoked = true
        URL.revokeObjectURL(url)
      },
    }
  }

  const url = file.url || (typeof file.source === 'string' ? file.source : '')
  if (!url) throw new Error('预览插件需要可读取的文件 URL。')
  return { url, revoke() {} }
}
