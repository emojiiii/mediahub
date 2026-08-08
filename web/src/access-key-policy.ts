import type { components } from './api/generated'

export const MAX_S3_IDENTITY_POLICY_BYTES = 20 * 1024

export type S3IdentityPolicyDocument = components['schemas']['S3IdentityPolicy']

export const ACCESS_KEY_POLICY_COPY = {
  action: '管理 S3 Policy',
  title: 'S3 Identity Policy',
  configured: '已配置',
  notConfigured: '未配置',
  defaultDeny: '未配置策略时默认拒绝所有 S3 请求。旧权限不会自动转换或参与 S3 授权。',
  template: '填入最小安全模板',
  templateHint: '模板通过显式 DenyAll 保持零授权；请按最小权限原则替换为明确的 Action 与 Resource，然后显式保存。',
  validationHint: '必须是有效 JSON，Version 使用 AWS Policy 版本，Statement 可以是单个对象或数组，且 UTF-8 大小不超过 20 KiB。服务端还会执行严格字段与重复键校验。',
  save: '保存 Policy',
  remove: '删除 Policy',
  close: '关闭',
  loading: '正在读取 Policy',
  saved: 'Policy 已保存',
  removed: 'Policy 已删除，当前 Access Key 的 S3 请求将默认拒绝。',
} as const

export const MINIMAL_S3_IDENTITY_POLICY: S3IdentityPolicyDocument = {
  Version: '2012-10-17',
  Statement: [{
    Sid: 'DenyAll',
    Effect: 'Deny',
    Action: 's3:*',
    Resource: '*',
  }],
}

export function formatS3IdentityPolicy(policy: S3IdentityPolicyDocument): string {
  return `${JSON.stringify(policy, null, 2)}\n`
}

export function validateS3IdentityPolicyJson(source: string):
  | { valid: true; policy: S3IdentityPolicyDocument; bytes: number }
  | { valid: false; message: string; bytes: number } {
  const bytes = new TextEncoder().encode(source).byteLength
  if (bytes > MAX_S3_IDENTITY_POLICY_BYTES) {
    return { valid: false, message: `Policy 超过 20 KiB 限制（当前 ${bytes} 字节）`, bytes }
  }

  let value: unknown
  try {
    value = JSON.parse(source)
  } catch (error) {
    const detail = error instanceof Error ? error.message : '未知语法错误'
    return { valid: false, message: `JSON 语法错误：${detail}`, bytes }
  }

  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return { valid: false, message: 'Policy 顶层必须是 JSON 对象', bytes }
  }

  const document = value as Record<string, unknown>
  if (document.Version !== '2012-10-17' && document.Version !== '2008-10-17') {
    return { valid: false, message: 'Version 必须是 2012-10-17 或 2008-10-17', bytes }
  }
  const statement = document.Statement
  if (!statement || typeof statement !== 'object') {
    return { valid: false, message: 'Statement 必须是单个对象或数组', bytes }
  }
  if (Array.isArray(statement) && statement.length === 0) {
    return { valid: false, message: 'Statement 数组至少需要一个声明；零授权请使用显式 DenyAll', bytes }
  }

  return { valid: true, policy: document as S3IdentityPolicyDocument, bytes }
}
