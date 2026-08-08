import { describe, expect, it } from 'vitest'

import {
  MAX_S3_IDENTITY_POLICY_BYTES,
  MINIMAL_S3_IDENTITY_POLICY,
  formatS3IdentityPolicy,
  validateS3IdentityPolicyJson,
} from './access-key-policy'

describe('S3 Identity Policy editor helpers', () => {
  it('uses an explicit deny-by-default template without granting actions', () => {
    expect(MINIMAL_S3_IDENTITY_POLICY).toEqual({
      Version: '2012-10-17',
      Statement: [{ Sid: 'DenyAll', Effect: 'Deny', Action: 's3:*', Resource: '*' }],
    })
    expect(formatS3IdentityPolicy(MINIMAL_S3_IDENTITY_POLICY)).toContain('"Effect": "Deny"')
  })

  it('validates the policy shape and returns parsed JSON', () => {
    const result = validateS3IdentityPolicyJson('{"Version":"2012-10-17","Statement":[{"Effect":"Deny","Action":"s3:*","Resource":"*"}]}')
    expect(result.valid).toBe(true)
    if (result.valid) expect(result.policy.Statement).toEqual([{ Effect: 'Deny', Action: 's3:*', Resource: '*' }])

    const single = validateS3IdentityPolicyJson('{"Version":"2012-10-17","Statement":{"Effect":"Deny","Action":"s3:*","Resource":"*"}}')
    expect(single.valid).toBe(true)
  })

  it('rejects malformed, structurally invalid, and oversized policy documents', () => {
    expect(validateS3IdentityPolicyJson('{').valid).toBe(false)
    expect(validateS3IdentityPolicyJson('{"Version":"2012-10-17","Statement":[]}')).toEqual(expect.objectContaining({ valid: false }))
    expect(validateS3IdentityPolicyJson('{"Version":"2012-10-17","Statement":"invalid"}')).toEqual(expect.objectContaining({ valid: false }))
    const oversized = JSON.stringify({ Version: '2012-10-17', Statement: [], padding: 'x'.repeat(MAX_S3_IDENTITY_POLICY_BYTES) })
    expect(validateS3IdentityPolicyJson(oversized)).toEqual(expect.objectContaining({ valid: false, bytes: expect.any(Number) }))
  })
})
