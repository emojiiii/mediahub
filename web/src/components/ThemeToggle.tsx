import { Monitor, Moon, Sun, type LucideIcon } from 'lucide-react'
import { useId } from 'react'

import { useTheme, type ThemePreference } from '../theme'

export interface ThemeToggleProps {
  className?: string
  disabled?: boolean
  label?: string
  showLabels?: boolean
}

interface ThemeOption {
  value: ThemePreference
  label: string
  description: string
  icon: LucideIcon
}

const THEME_OPTIONS: ThemeOption[] = [
  { value: 'light', label: '浅色', description: '使用浅色主题', icon: Sun },
  { value: 'dark', label: '深色', description: '使用深色主题', icon: Moon },
  { value: 'system', label: '跟随系统', description: '跟随系统外观设置', icon: Monitor },
]

export function ThemeToggle({
  className = '',
  disabled = false,
  label = '界面主题',
  showLabels = true,
}: ThemeToggleProps) {
  const { theme, setTheme } = useTheme()
  const groupName = useId()

  return (
    <fieldset className={`m-0 min-w-0 border-0 p-0 ${className}`.trim()} disabled={disabled}>
      <legend className="sr-only">{label}</legend>
      <div className="inline-flex items-center gap-1 rounded-lg border border-separator bg-default-soft p-1">
        {THEME_OPTIONS.map(({ value, label: optionLabel, description, icon: Icon }) => {
          const selected = theme === value
          return (
            <label
              className={`relative ${disabled ? 'cursor-not-allowed opacity-55' : 'cursor-pointer'}`}
              key={value}
              title={description}
            >
              <input
                checked={selected}
                className="peer sr-only"
                name={groupName}
                onChange={() => setTheme(value)}
                type="radio"
                value={value}
              />
              <span
                className={`flex h-8 items-center justify-center gap-1.5 rounded-md text-xs font-medium transition-colors peer-focus-visible:outline peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-focus ${
                  showLabels ? 'px-2.5' : 'w-8'
                } ${
                  selected
                    ? 'bg-surface text-foreground shadow-sm ring-1 ring-inset ring-separator'
                    : 'text-muted hover:bg-default-soft-hover hover:text-foreground'
                }`}
              >
                <Icon aria-hidden="true" className="size-4 shrink-0" />
                <span className={showLabels ? '' : 'sr-only'}>{optionLabel}</span>
              </span>
            </label>
          )
        })}
      </div>
    </fieldset>
  )
}
