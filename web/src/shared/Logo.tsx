import { useId } from 'react'

/** The logo of `docs/assets` (sound bars and rising dots, then the word) in the brand gradient. */
export function Logo({ className = '' }: { className?: string }) {
  const gradient = useId()
  return (
    <span className={`inline-flex items-center gap-2 font-bold tracking-tight ${className}`}>
      <svg viewBox="4 6 100 80" className="h-[1.1em] w-auto" aria-hidden="true">
        <defs>
          <linearGradient id={gradient} gradientUnits="userSpaceOnUse" x1="8" x2="100" y1="0" y2="0">
            <stop offset="0" stopColor="rgb(0 150 190)" />
            <stop offset="1" stopColor="rgb(140 90 230)" />
          </linearGradient>
        </defs>
        <path
          d="M12 38v20M26 26v44M40 16v64M54 30v36M68 40v16"
          stroke={`url(#${gradient})`}
          strokeWidth="8"
          strokeLinecap="round"
        />
        <g fill={`url(#${gradient})`}>
          <circle cx="80" cy="34" r="4.5" />
          <circle cx="90" cy="22" r="3.5" />
          <circle cx="98" cy="12" r="2.5" />
        </g>
      </svg>
      <span>liberarr</span>
    </span>
  )
}
