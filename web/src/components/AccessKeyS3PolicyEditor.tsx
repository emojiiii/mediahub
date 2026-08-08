import { Button, Chip, Modal as HeroModal, Spinner, TextArea } from '@heroui/react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { LoaderCircle, ShieldCheck, Trash2 } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'

import {
  ACCESS_KEY_POLICY_COPY,
  MINIMAL_S3_IDENTITY_POLICY,
  formatS3IdentityPolicy,
  validateS3IdentityPolicyJson,
} from '../access-key-policy'
import { api, errorMessage, type AccessKey, type S3IdentityPolicyDocument } from '../api'

type AccessKeyS3PolicyEditorProps = {
  accessKey: AccessKey
  onClose: () => void
}

export function AccessKeyS3PolicyEditor({ accessKey, onClose }: AccessKeyS3PolicyEditorProps) {
  const queryClient = useQueryClient()
  const queryKey = ['access-key-s3-policy', accessKey.id] as const
  const policy = useQuery({ queryKey, queryFn: () => api.getAccessKeyS3Policy(accessKey.id), retry: false })
  const [source, setSource] = useState('')
  const [notice, setNotice] = useState<string | null>(null)

  useEffect(() => {
    if (policy.data !== undefined) setSource(policy.data ? formatS3IdentityPolicy(policy.data) : '')
  }, [policy.data])

  const validation = useMemo(() => validateS3IdentityPolicyJson(source), [source])
  const save = useMutation<S3IdentityPolicyDocument, Error, S3IdentityPolicyDocument>({
    mutationFn: (document) => api.putAccessKeyS3Policy(accessKey.id, document),
    onSuccess: (document) => {
      queryClient.setQueryData(queryKey, document)
      setSource(formatS3IdentityPolicy(document))
      setNotice(ACCESS_KEY_POLICY_COPY.saved)
    },
  })
  const remove = useMutation<void, Error>({
    mutationFn: () => api.deleteAccessKeyS3Policy(accessKey.id),
    onSuccess: () => {
      queryClient.setQueryData(queryKey, null)
      setSource('')
      setNotice(ACCESS_KEY_POLICY_COPY.removed)
    },
  })
  const pending = save.isPending || remove.isPending
  const configured = policy.data !== null && policy.data !== undefined
  const mutationError = save.error ?? remove.error

  const fillTemplate = () => {
    setSource(formatS3IdentityPolicy(MINIMAL_S3_IDENTITY_POLICY))
    setNotice(null)
  }
  const submit = () => {
    setNotice(null)
    if (validation.valid) save.mutate(validation.policy)
  }
  const deletePolicy = () => {
    if (window.confirm(`删除 ${accessKey.name} 的 S3 Identity Policy？删除后所有 S3 请求默认拒绝。`)) {
      setNotice(null)
      remove.mutate()
    }
  }

  return <HeroModal isOpen onOpenChange={(open) => { if (!open && !pending) onClose() }}>
    <HeroModal.Backdrop isDismissable={!pending} variant="blur">
      <HeroModal.Container placement="center" scroll="inside" size="lg">
        <HeroModal.Dialog aria-label={ACCESS_KEY_POLICY_COPY.title}>
          <HeroModal.Header>
            <HeroModal.Heading>{ACCESS_KEY_POLICY_COPY.title}</HeroModal.Heading>
            {!pending && <HeroModal.CloseTrigger />}
          </HeroModal.Header>
          <HeroModal.Body>
            {policy.isLoading ? <div className="grid min-h-52 place-items-center"><Spinner aria-label={ACCESS_KEY_POLICY_COPY.loading} color="accent" /></div> : <div className="space-y-4">
              <div className="flex flex-col gap-3 rounded-md border border-separator bg-default-soft px-4 py-3 sm:flex-row sm:items-start sm:justify-between">
                <div className="min-w-0">
                  <div className="flex items-center gap-2"><ShieldCheck className="size-4 text-accent" /><span className="text-sm font-semibold">{accessKey.name}</span><Chip size="sm" variant="soft"><Chip.Label>{configured ? ACCESS_KEY_POLICY_COPY.configured : ACCESS_KEY_POLICY_COPY.notConfigured}</Chip.Label></Chip></div>
                  <code className="mt-1 block truncate text-[11px] text-muted" title={accessKey.id}>{accessKey.id}</code>
                  <p className="mt-2 text-xs leading-5 text-muted">{ACCESS_KEY_POLICY_COPY.defaultDeny}</p>
                </div>
                <Button size="sm" variant="secondary" className="shrink-0" isDisabled={pending} onClick={fillTemplate}>{ACCESS_KEY_POLICY_COPY.template}</Button>
              </div>

              <label>
                <span className="mb-1.5 block text-xs font-medium text-muted">Policy JSON</span>
                <TextArea
                  fullWidth
                  aria-label="S3 Identity Policy JSON"
                  className="min-h-80 font-mono text-xs leading-5"
                  spellCheck={false}
                  placeholder="点击“填入最小安全模板”开始；内容不会自动保存。"
                  value={source}
                  onChange={(event) => { setSource(event.target.value); setNotice(null) }}
                />
              </label>
              <div className="flex flex-col gap-1 text-xs leading-5 text-muted sm:flex-row sm:items-start sm:justify-between">
                <p>{ACCESS_KEY_POLICY_COPY.validationHint}</p>
                <span className="shrink-0 font-mono tabular-nums">{validation.bytes} / 20480 B</span>
              </div>
              <p className="text-xs leading-5 text-muted">{ACCESS_KEY_POLICY_COPY.templateHint}</p>
              {source && !validation.valid && <p role="alert" className="rounded-md border border-danger/25 bg-danger-soft px-3 py-2 text-sm text-danger-soft-foreground">{validation.message}</p>}
              {policy.error && <p role="alert" className="rounded-md border border-danger/25 bg-danger-soft px-3 py-2 text-sm text-danger-soft-foreground">{errorMessage(policy.error)}</p>}
              {mutationError && <p role="alert" className="rounded-md border border-danger/25 bg-danger-soft px-3 py-2 text-sm text-danger-soft-foreground">{errorMessage(mutationError)}</p>}
              {notice && <p role="status" className="rounded-md border border-success/25 bg-success-soft px-3 py-2 text-sm text-success-soft-foreground">{notice}</p>}

              <div className="flex flex-col-reverse gap-2 border-t border-separator pt-4 sm:flex-row sm:items-center sm:justify-between">
                <Button variant="danger-soft" size="sm" isDisabled={!configured || pending} onClick={deletePolicy}><Trash2 className="size-4" />{ACCESS_KEY_POLICY_COPY.remove}</Button>
                <div className="flex justify-end gap-2"><Button variant="secondary" isDisabled={pending} onClick={onClose}>{ACCESS_KEY_POLICY_COPY.close}</Button><Button variant="primary" isDisabled={!validation.valid || pending} onClick={submit}>{save.isPending && <LoaderCircle className="size-4 animate-spin" />}{ACCESS_KEY_POLICY_COPY.save}</Button></div>
              </div>
            </div>}
          </HeroModal.Body>
        </HeroModal.Dialog>
      </HeroModal.Container>
    </HeroModal.Backdrop>
  </HeroModal>
}
