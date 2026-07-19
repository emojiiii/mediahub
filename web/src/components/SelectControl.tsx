import { Dropdown } from '@heroui/react'
import { Check, ChevronDown } from 'lucide-react'

export type SelectControlOption = {
  value: string
  label: string
  description?: string
}

export type SelectControlProps = {
  'aria-label': string
  value: string
  options: readonly SelectControlOption[]
  onChange: (value: string) => void
  className?: string
  isDisabled?: boolean
}

const EMPTY_KEY = '__mediahub_empty_value__'

export function SelectControl({
  'aria-label': ariaLabel,
  value,
  options,
  onChange,
  className,
  isDisabled = false,
}: SelectControlProps) {
  const selected = options.find((option) => option.value === value) ?? options[0]
  const selectedKey = selected?.value || EMPTY_KEY

  return (
    <Dropdown>
      <Dropdown.Trigger
        aria-label={ariaLabel}
        className={[
          'group flex h-10 min-w-0 items-center gap-2 rounded-md border border-field-border bg-field px-3 text-left text-sm text-foreground outline-none transition',
          'hover:border-border-secondary hover:bg-field-hover focus-visible:border-field-border-focus focus-visible:ring-2 focus-visible:ring-focus/15',
          'disabled:cursor-not-allowed disabled:opacity-55',
          className,
        ].filter(Boolean).join(' ')}
        isDisabled={isDisabled}
      >
        <span className="min-w-0 flex-1 truncate">{selected?.label ?? '请选择'}</span>
        <ChevronDown aria-hidden="true" className="size-4 shrink-0 text-muted transition-transform group-aria-expanded:rotate-180" />
      </Dropdown.Trigger>
      <Dropdown.Popover className="min-w-44 overflow-hidden rounded-lg border border-separator-secondary bg-overlay p-1 shadow-[var(--overlay-shadow)]" offset={6} placement="bottom start">
        <Dropdown.Menu
          aria-label={ariaLabel}
          className="max-h-72 overflow-y-auto outline-none"
          onAction={(key) => onChange(String(key) === EMPTY_KEY ? '' : String(key))}
          selectedKeys={new Set([selectedKey])}
          selectionMode="single"
        >
          {options.map((option) => {
            const key = option.value || EMPTY_KEY
            const isSelected = option.value === value
            return <Dropdown.Item id={key} key={key} className="flex min-h-9 cursor-default items-center gap-2 rounded-md px-2.5 py-2 text-sm text-foreground outline-none hover:bg-default-soft focus:bg-default-soft selected:bg-accent-soft" textValue={option.label}>
              <span className="min-w-0 flex-1"><span className="block truncate">{option.label}</span>{option.description && <span className="mt-0.5 block truncate text-[10px] text-muted">{option.description}</span>}</span>
              <span className="grid size-5 shrink-0 place-items-center text-accent">{isSelected && <Check aria-hidden="true" className="size-3.5" />}</span>
            </Dropdown.Item>
          })}
        </Dropdown.Menu>
      </Dropdown.Popover>
    </Dropdown>
  )
}
