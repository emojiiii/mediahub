export const ARCHIVE_MAX_PASSWORD_LENGTH = 1_024

export type ArchiveScanRequest = {
  type: 'scan'
  url: string
  sourceSize: number
  password?: string
}

export type ArchiveExportRequest = {
  type: 'export'
  url: string
  sourceSize: number
  password?: string
  path: string
  target: 'file' | 'folder'
}

export type ArchiveWorkerRequest = ArchiveScanRequest | ArchiveExportRequest

export type ArchiveEntrySummary = {
  path: string
  size: number
  directory: boolean
}

export type ArchiveScanResult = {
  entries: ArchiveEntrySummary[]
  entryCount: number
  totalDeclaredSize: number
  truncated: boolean
  encrypted: boolean | null
  truncationReason?: string
}

export type ArchiveWorkerScanSuccess = {
  type: 'scan-success'
  result: ArchiveScanResult
}

export type ArchiveWorkerPasswordRequired = {
  type: 'password-required'
  invalid: boolean
  error: string
  result?: ArchiveScanResult
}

export type ArchiveWorkerExportSuccess = {
  type: 'export-success'
  path: string
  fileName: string
  mimeType: string
  data: ArrayBuffer
}

export type ArchiveWorkerFailure = {
  type: 'error'
  operation: 'scan' | 'export'
  error: string
  path?: string
}

export type ArchiveWorkerResponse =
  | ArchiveWorkerScanSuccess
  | ArchiveWorkerPasswordRequired
  | ArchiveWorkerExportSuccess
  | ArchiveWorkerFailure

export function isArchiveWorkerRequest(value: unknown): value is ArchiveWorkerRequest {
  if (!value || typeof value !== 'object') return false
  const request = value as Partial<ArchiveWorkerRequest>
  if (!validSource(request.url, request.sourceSize) || !validPassword(request.password)) return false
  if (request.type === 'scan') return true
  return request.type === 'export'
    && (request.target === 'file' || request.target === 'folder')
    && typeof request.path === 'string'
    && request.path.length > 0
    && request.path.length <= 1_024
}

function validSource(url: unknown, sourceSize: unknown): boolean {
  return typeof url === 'string' && url.length > 0 && typeof sourceSize === 'number'
}

function validPassword(password: unknown): boolean {
  return password === undefined
    || (typeof password === 'string' && password.length <= ARCHIVE_MAX_PASSWORD_LENGTH && !password.includes('\0'))
}
