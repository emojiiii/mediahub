import JSZip from 'jszip'

export type ArchiveFolderFile = {
  path: string
  data: Uint8Array
}

export async function createStoredArchiveFolderZip(
  files: readonly ArchiveFolderFile[],
  folderPath: string,
): Promise<Uint8Array> {
  const prefix = `${folderPath}/`
  const archive = new JSZip()

  for (const entry of files) {
    if (!entry.path.startsWith(prefix)) {
      throw new Error('Archive folder export contains an entry outside the selected folder')
    }
    const relativePath = entry.path.slice(prefix.length)
    if (!relativePath) {
      throw new Error('Archive folder export contains an empty relative path')
    }
    const data = entry.data.buffer.slice(
      entry.data.byteOffset,
      entry.data.byteOffset + entry.data.byteLength,
    ) as ArrayBuffer
    archive.file(`${folderPath}/${relativePath}`, data, {
      binary: true,
      compression: 'STORE',
      createFolders: true,
    })
  }

  return archive.generateAsync({
    type: 'uint8array',
    compression: 'STORE',
    platform: 'DOS',
    streamFiles: true,
  })
}
