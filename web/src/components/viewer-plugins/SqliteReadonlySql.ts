const READ_ONLY_PRAGMAS = new Set([
  'application_id',
  'collation_list',
  'compile_options',
  'database_list',
  'encoding',
  'foreign_key_check',
  'foreign_key_list',
  'freelist_count',
  'function_list',
  'index_info',
  'index_list',
  'index_xinfo',
  'integrity_check',
  'journal_mode',
  'module_list',
  'page_count',
  'page_size',
  'pragma_list',
  'quick_check',
  'schema_version',
  'table_info',
  'table_list',
  'table_xinfo',
  'user_version',
])

const READ_ONLY_PRAGMAS_WITH_ARGUMENTS = new Set([
  'foreign_key_check',
  'foreign_key_list',
  'index_info',
  'index_list',
  'index_xinfo',
  'integrity_check',
  'quick_check',
  'table_info',
  'table_xinfo',
])

const WRITE_OR_CONTROL_KEYWORDS = new Set([
  'ALTER', 'ANALYZE', 'ATTACH', 'BEGIN', 'COMMIT', 'CREATE', 'DELETE', 'DETACH', 'DROP', 'END',
  'INSERT', 'LOAD_EXTENSION', 'REINDEX', 'RELEASE', 'REPLACE', 'ROLLBACK', 'SAVEPOINT', 'UPDATE',
  'VACUUM',
])

export type ReadonlySqlValidation =
  | { valid: true; sql: string }
  | { valid: false; error: string }

type SqlToken = {
  kind: 'word' | 'semicolon' | 'symbol'
  value: string
}

export function validateReadonlySql(source: string): ReadonlySqlValidation {
  const sql = source.replace(/^\uFEFF/, '').trim()
  if (!sql) return invalid('请输入要执行的 SQL。')

  const tokenized = tokenizeSql(sql)
  if (!tokenized.valid) return tokenized
  const tokens = tokenized.tokens
  const semicolons = tokens.reduce((count, token) => count + Number(token.kind === 'semicolon'), 0)
  if (semicolons > 1) return invalid('一次只能执行一条只读 SQL。')
  if (semicolons === 1 && tokens[tokens.length - 1]?.kind !== 'semicolon') return invalid('一次只能执行一条只读 SQL。')

  const words = tokens.filter((token) => token.kind === 'word').map((token) => token.value.toUpperCase())
  const first = words[0]
  if (!first) return invalid('请输入有效的 SQL。')

  for (const keyword of words) {
    if (WRITE_OR_CONTROL_KEYWORDS.has(keyword)) return invalid(`不允许执行 ${keyword} 语句。`)
  }

  if (first === 'SELECT') return { valid: true, sql }
  if (first === 'WITH') {
    if (!words.includes('SELECT')) return invalid('WITH 语句必须以只读 SELECT 查询返回结果。')
    return { valid: true, sql }
  }
  if (first === 'EXPLAIN') {
    const explained = words[1] === 'QUERY' && words[2] === 'PLAN' ? words[3] : words[1]
    if (explained !== 'SELECT' && explained !== 'WITH') {
      return invalid('EXPLAIN 只能用于 SELECT 或只读 WITH 查询。')
    }
    return { valid: true, sql }
  }
  if (first === 'PRAGMA') return validatePragma(sql, words, tokens)
  return invalid('仅支持 SELECT、只读 WITH、EXPLAIN 和白名单 PRAGMA 查询。')
}

function validatePragma(sql: string, words: string[], tokens: SqlToken[]): ReadonlySqlValidation {
  if (tokens.some((token) => token.kind === 'symbol' && token.value === '=')) return invalid('不允许通过 PRAGMA 修改数据库设置。')
  const pragmaName = (words[2] && sqlAfterPragmaStartsWithSchema(sql)) ? words[2] : words[1]
  if (!pragmaName || !READ_ONLY_PRAGMAS.has(pragmaName.toLowerCase())) {
    return invalid('该 PRAGMA 不在只读查询白名单中。')
  }
  if (tokens.some((token) => token.kind === 'symbol' && token.value === '(') && !READ_ONLY_PRAGMAS_WITH_ARGUMENTS.has(pragmaName.toLowerCase())) {
    return invalid('不允许通过 PRAGMA 参数修改数据库设置。')
  }
  return { valid: true, sql }
}

function sqlAfterPragmaStartsWithSchema(sql: string): boolean {
  const body = sql.replace(/^\s*PRAGMA\s+/i, '')
  return /^[A-Za-z_][\w$]*\s*\./.test(body)
}

function tokenizeSql(sql: string): { valid: true; tokens: SqlToken[] } | { valid: false; error: string } {
  const tokens: SqlToken[] = []
  let index = 0
  while (index < sql.length) {
    const character = sql[index]
    const next = sql[index + 1]
    if (/\s/.test(character)) {
      index += 1
      continue
    }
    if (character === '-' && next === '-') {
      index += 2
      while (index < sql.length && sql[index] !== '\n') index += 1
      continue
    }
    if (character === '/' && next === '*') {
      const closing = sql.indexOf('*/', index + 2)
      if (closing < 0) return invalid('SQL 块注释没有闭合。')
      index = closing + 2
      continue
    }
    if (character === "'" || character === '"' || character === '`') {
      const closing = consumeQuoted(sql, index, character)
      if (closing < 0) return invalid('SQL 中存在未闭合的引号。')
      index = closing
      continue
    }
    if (character === '[') {
      const closing = sql.indexOf(']', index + 1)
      if (closing < 0) return invalid('SQL 中存在未闭合的标识符。')
      index = closing + 1
      continue
    }
    if (character === ';') {
      tokens.push({ kind: 'semicolon', value: character })
      index += 1
      continue
    }
    if (/[A-Za-z_]/.test(character)) {
      let end = index + 1
      while (end < sql.length && /[\w$]/.test(sql[end])) end += 1
      tokens.push({ kind: 'word', value: sql.slice(index, end) })
      index = end
      continue
    }
    tokens.push({ kind: 'symbol', value: character })
    index += 1
  }
  return { valid: true, tokens }
}

function consumeQuoted(sql: string, start: number, quote: string): number {
  let index = start + 1
  while (index < sql.length) {
    if (sql[index] !== quote) {
      index += 1
      continue
    }
    if (sql[index + 1] === quote) {
      index += 2
      continue
    }
    return index + 1
  }
  return -1
}

function invalid(error: string): { valid: false; error: string } {
  return { valid: false, error }
}
