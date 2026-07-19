import { describe, expect, it } from 'vitest'

import { validateReadonlySql } from './SqliteReadonlySql'

describe('validateReadonlySql', () => {
  it.each([
    'SELECT * FROM users',
    "SELECT '; DROP TABLE users' AS value; -- DROP is data here",
    'WITH active AS (SELECT * FROM users) SELECT * FROM active',
    'WITH RECURSIVE counter(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM counter WHERE x < 3) SELECT * FROM counter',
    'EXPLAIN SELECT * FROM users',
    'EXPLAIN QUERY PLAN WITH sample AS (SELECT 1) SELECT * FROM sample',
    'PRAGMA table_info(users)',
    'PRAGMA table_info("audit=events")',
    'PRAGMA main.table_xinfo("users")',
    'PRAGMA integrity_check;',
  ])('allows a single read-only statement: %s', (sql) => {
    expect(validateReadonlySql(sql)).toEqual({ valid: true, sql: sql.trim() })
  })

  it.each([
    ['SELECT 1; SELECT 2', '一次只能执行一条'],
    ['SELECT 1;;', '一次只能执行一条'],
    ['WITH changed AS (DELETE FROM users RETURNING *) SELECT * FROM changed', 'DELETE'],
    ['EXPLAIN UPDATE users SET name = \'x\'', 'UPDATE'],
    ['INSERT INTO users VALUES (1)', 'INSERT'],
    ['CREATE TABLE unsafe(id)', 'CREATE'],
    ["ATTACH DATABASE 'other.db' AS other", 'ATTACH'],
    ['DETACH DATABASE other', 'DETACH'],
    ['VACUUM', 'VACUUM'],
    ['PRAGMA user_version = 2', '不允许通过 PRAGMA 修改'],
    ['PRAGMA user_version(2)', '不允许通过 PRAGMA 参数修改'],
    ['PRAGMA main.journal_mode(WAL)', '不允许通过 PRAGMA 参数修改'],
    ['PRAGMA page_size(4096)', '不允许通过 PRAGMA 参数修改'],
    ['PRAGMA writable_schema', '不在只读查询白名单'],
    ['/* missing', '注释没有闭合'],
  ])('rejects unsafe SQL: %s', (sql, reason) => {
    const result = validateReadonlySql(sql)
    expect(result.valid).toBe(false)
    if (!result.valid) expect(result.error).toContain(reason)
  })

  it('does not treat keywords and semicolons inside quoted values or comments as statements', () => {
    expect(validateReadonlySql('SELECT "update", `drop`, [delete], \'a;b\' /* ; UPDATE */;')).toMatchObject({ valid: true })
  })
})
