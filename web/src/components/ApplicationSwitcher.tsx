import { Dropdown, Spinner } from '@heroui/react'
import { Check, ChevronDown, Layers3, Plus } from 'lucide-react'

export interface ApplicationSwitcherItem {
  appId: string
  name: string
}

export interface ApplicationSwitcherProps {
  applications: readonly ApplicationSwitcherItem[]
  currentAppId: string
  onSelect: (appId: string) => void
  onCreate: () => void
  isLoading?: boolean
  isDisabled?: boolean
  className?: string
}

const CREATE_APPLICATION_KEY = '__create_application__'

export function shortAppId(appId: string): string {
  if (appId.length <= 22) return appId
  return `${appId.slice(0, 14)}...${appId.slice(-5)}`
}

export function ApplicationSwitcher({
  applications,
  currentAppId,
  onSelect,
  onCreate,
  isLoading = false,
  isDisabled = false,
  className,
}: ApplicationSwitcherProps) {
  const currentApplication = applications.find((application) => application.appId === currentAppId)
  const isUnavailable = isDisabled || isLoading
  const currentName = currentApplication?.name ?? (applications.length ? '选择应用' : '暂无应用')
  const currentId = currentApplication?.appId ?? currentAppId

  return (
    <Dropdown>
      <Dropdown.Trigger
        aria-label={`切换应用，当前为 ${currentName}`}
        className={[
          'group flex min-h-14 w-full items-center gap-3 rounded-lg border border-white/10 bg-white/[0.055] px-3 py-2 text-left text-white outline-none transition',
          'hover:border-white/20 hover:bg-white/[0.09] pressed:bg-white/[0.12] focus-visible:border-white/30 focus-visible:ring-2 focus-visible:ring-white/20',
          'disabled:cursor-not-allowed disabled:opacity-55',
          className,
        ].filter(Boolean).join(' ')}
        isDisabled={isUnavailable}
      >
        <span className="grid size-8 shrink-0 place-items-center rounded-md bg-white/10 text-white/75 ring-1 ring-inset ring-white/10">
          {isLoading ? (
            <Spinner aria-label="正在加载应用" color="current" size="sm" />
          ) : (
            <Layers3 aria-hidden="true" className="size-4" />
          )}
        </span>

        <span className="min-w-0 flex-1">
          <span className="block truncate text-sm font-semibold leading-5">{isLoading ? '正在加载应用' : currentName}</span>
          <span className="block truncate font-mono text-[11px] leading-4 text-white/45">
            {isLoading ? '请稍候' : currentId ? shortAppId(currentId) : '创建首个应用'}
          </span>
        </span>

        <ChevronDown
          aria-hidden="true"
          className="size-4 shrink-0 text-white/45 transition-transform group-aria-expanded:rotate-180 group-hover:text-white/75"
        />
      </Dropdown.Trigger>

      <Dropdown.Popover
        className="min-w-64 overflow-hidden rounded-lg border border-white/10 bg-[#151a20] p-1 shadow-2xl shadow-black/35"
        offset={8}
        placement="bottom start"
      >
        <Dropdown.Menu
          aria-label="选择应用"
          className="max-h-80 overflow-y-auto outline-none"
          onAction={(key) => {
            const value = String(key)
            if (value === CREATE_APPLICATION_KEY) {
              onCreate()
              return
            }
            if (value !== currentAppId) onSelect(value)
          }}
          selectedKeys={currentApplication ? new Set([currentApplication.appId]) : new Set()}
          selectionMode="single"
        >
          {applications.map((application) => {
            const isCurrent = application.appId === currentAppId
            return (
              <Dropdown.Item
                id={application.appId}
                key={application.appId}
                className="group/item flex min-h-12 cursor-default items-center gap-3 rounded-md px-2.5 py-2 text-white outline-none selected:bg-white/[0.08] hover:bg-white/[0.07] focus:bg-white/[0.1]"
                textValue={`${application.name} ${application.appId}`}
              >
                <span className="grid size-7 shrink-0 place-items-center rounded-md bg-white/[0.07] text-xs font-semibold uppercase text-white/70 ring-1 ring-inset ring-white/10">
                  {application.name.trim().slice(0, 1) || 'A'}
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm font-medium leading-5 text-white/90">{application.name}</span>
                  <span className="block truncate font-mono text-[11px] leading-4 text-white/40">{shortAppId(application.appId)}</span>
                </span>
                <span className="grid size-6 shrink-0 place-items-center text-[#73dfa3]">
                  {isCurrent && <Check aria-hidden="true" className="size-4" />}
                </span>
              </Dropdown.Item>
            )
          })}

          <Dropdown.Item
            id={CREATE_APPLICATION_KEY}
            className="mt-1 flex min-h-10 cursor-default items-center gap-2.5 rounded-md border-t border-white/10 px-2.5 pt-2 text-sm font-medium text-white/75 outline-none hover:bg-white/[0.07] hover:text-white focus:bg-white/[0.1] focus:text-white"
            textValue="新建应用"
          >
            <span className="grid size-7 shrink-0 place-items-center rounded-md bg-[#6d5dfc]/20 text-[#a99fff]">
              <Plus aria-hidden="true" className="size-4" />
            </span>
            <span>新建应用</span>
          </Dropdown.Item>
        </Dropdown.Menu>
      </Dropdown.Popover>
    </Dropdown>
  )
}
