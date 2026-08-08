export interface PrismArkMarkProps {
  className?: string
  decorative?: boolean
  title?: string
}

/** PrismArk 的 imagegen 正式品牌标记。 */
export function PrismArkMark({
  className = 'size-10',
  decorative = true,
  title = 'PrismArk',
}: PrismArkMarkProps) {
  return (
    <img
      src="/brand/prismark-mark-64.png"
      srcSet="/brand/prismark-mark-64.png 1x, /brand/prismark-mark-192.png 2x"
      width="64"
      height="64"
      className={className}
      alt={decorative ? '' : title}
      aria-hidden={decorative || undefined}
      decoding="async"
    />
  )
}

export interface PrismArkBrandProps {
  className?: string
  compact?: boolean
}

export function PrismArkBrand({ className = '', compact = false }: PrismArkBrandProps) {
  return (
    <span className={`inline-flex items-center gap-2.5 ${className}`}>
      <PrismArkMark className={compact ? 'size-8' : 'size-9'} />
      <span className="flex flex-col text-left leading-none">
        <span className="text-base font-bold text-foreground">PrismArk</span>
        {!compact && <span className="mt-1 text-[10px] font-medium text-muted">万象仓</span>}
      </span>
    </span>
  )
}
