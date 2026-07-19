import { describe, expect, it } from 'vitest'

import { createMediaHubSpreadsheetPlugin, isSpreadsheetFile } from './OpenFileViewerSpreadsheetPlugin'

describe('OpenFileViewerSpreadsheetPlugin', () => {
  it('matches common spreadsheet extensions before the upstream office plugin', () => {
    for (const extension of ['csv', '.TSV', 'xls', 'xlsx', 'XLSM', 'xlsb', 'ods']) {
      expect(isSpreadsheetFile({ extension, mimeType: 'application/octet-stream' })).toBe(true)
    }
    expect(isSpreadsheetFile({ extension: 'docx', mimeType: 'application/octet-stream' })).toBe(false)
  })

  it('matches spreadsheet MIME types with optional parameters', () => {
    expect(isSpreadsheetFile({ extension: '', mimeType: 'text/csv; charset=utf-8' })).toBe(true)
    expect(isSpreadsheetFile({ extension: '', mimeType: 'text/tab-separated-values' })).toBe(true)
    expect(isSpreadsheetFile({ extension: '', mimeType: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet' })).toBe(true)
    expect(isSpreadsheetFile({ extension: '', mimeType: 'application/vnd.ms-excel.sheet.macroEnabled.12' })).toBe(true)
  })

  it('exposes a dedicated plugin for ObjectFileViewer integration', () => {
    const plugin = createMediaHubSpreadsheetPlugin(4096)
    expect(plugin.name).toBe('mediahub-spreadsheet')
    expect(plugin.match({ extension: 'xlsx', mimeType: 'application/octet-stream' } as never)).toBe(true)
  })
})
