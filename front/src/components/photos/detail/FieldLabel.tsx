import type {ReactNode} from 'react'

/** Small green chip used for EXIF/metadata field names so they stand out from values & subtitles. */
export function FieldLabel({children}: { children: ReactNode }) {
    return (
        <span className="inline-flex shrink-0 items-center rounded bg-emerald-500/15 px-1.5 py-0.5 text-[11px] font-medium text-emerald-400">
            {children}
        </span>
    )
}
