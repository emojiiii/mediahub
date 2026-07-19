import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url))
const schemaPath = path.resolve(scriptDirectory, '../../openapi/openapi.json')
const document = JSON.parse(await readFile(schemaPath, 'utf8'))
const methods = new Set(['get', 'put', 'post', 'delete', 'options', 'head', 'patch', 'trace'])
let operationCount = 0
let referenceCount = 0

function resolveLocalReference(reference) {
  if (!reference.startsWith('#/')) throw new Error(`Only local OpenAPI references are allowed: ${reference}`)
  return reference.slice(2).split('/').reduce((value, token) => {
    const key = token.replaceAll('~1', '/').replaceAll('~0', '~')
    if (value === null || typeof value !== 'object' || !(key in value)) {
      throw new Error(`Unresolved OpenAPI reference: ${reference}`)
    }
    return value[key]
  }, document)
}

function visit(value) {
  if (Array.isArray(value)) {
    value.forEach(visit)
    return
  }
  if (value === null || typeof value !== 'object') return
  if (typeof value.$ref === 'string') {
    resolveLocalReference(value.$ref)
    referenceCount += 1
  }
  Object.values(value).forEach(visit)
}

if (!document.openapi?.startsWith('3.')) throw new Error('OpenAPI document must use version 3.x')
if (document.paths === null || typeof document.paths !== 'object') throw new Error('OpenAPI paths are missing')

for (const [route, pathItem] of Object.entries(document.paths)) {
  for (const [method, operation] of Object.entries(pathItem)) {
    if (!methods.has(method)) continue
    operationCount += 1
    if (!Object.hasOwn(operation, 'security') || !Array.isArray(operation.security)) {
      throw new Error(`${method.toUpperCase()} ${route} must declare security explicitly`)
    }
  }
}

visit(document)
console.log(`OpenAPI verified: ${Object.keys(document.paths).length} paths, ${operationCount} operations, ${referenceCount} local references.`)
