import {memo, type MouseEvent} from 'react'
import {Check} from 'lucide-react'
import type {PictureListItem} from '@/lib/types'
import {cn} from '@/lib/utils'
import {Blurhash} from './Blurhash'

interface PhotoCardProps {
    item: PictureListItem
    /** Baseline row height (px); flex-basis is derived from it and the aspect ratio. */
    rowHeight: number
    selected: boolean
    onSelect: (event: MouseEvent) => void
    onOpen: () => void
}

/**
 * One justified-grid cell. Following the reference CSS: flex-basis/flex-grow are
 * set from the picture's aspect ratio (basis = rowHeight × ratio), and the cell
 * carries the picture's `aspect-ratio` so flexbox derives a uniform row height
 * while preserving each picture's shape — no cropping, minimal JS.
 */
export const PhotoCard = memo(function PhotoCard({item, rowHeight, selected, onSelect, onOpen}: PhotoCardProps) {
    const ratio = item.width && item.height ? item.width / item.height : 1
    const basis = rowHeight * ratio

    return (
        <li
            style={{
                flexBasis: `${basis}px`,
                flexGrow: basis,
                aspectRatio: `${item.width ?? 1} / ${item.height ?? 1}`,
            }}
            className={cn(
                'group relative cursor-pointer overflow-hidden rounded-md bg-muted',
                selected && 'ring-2 ring-primary ring-offset-2 ring-offset-background',
            )}
            onClick={onSelect}
            onDoubleClick={onOpen}
        >
            {item.blurhash && <Blurhash hash={item.blurhash} className="absolute inset-0 h-full w-full"/>}

            {item.thumbnail_url && (
                <img
                    src={item.thumbnail_url}
                    alt={item.filename ?? ''}
                    loading="lazy"
                    className="absolute inset-0 h-full w-full object-cover opacity-0 transition-opacity duration-200"
                    onLoad={(e) => {
                        e.currentTarget.style.opacity = '1'
                    }}
                />
            )}

            <div
                className={cn(
                    'absolute left-2 top-2 flex h-5 w-5 items-center justify-center rounded-full border border-white/70 bg-black/40 text-white opacity-0 transition-opacity group-hover:opacity-100',
                    selected && 'border-primary bg-primary text-primary-foreground opacity-100',
                )}
            >
                {selected && <Check className="h-3.5 w-3.5"/>}
            </div>

            {!item.owned && item.owner_username && (
                <span className="absolute bottom-1 left-1 rounded bg-black/55 px-1 text-[10px] leading-4 text-white">
          @{item.owner_username}
        </span>
            )}
        </li>
    )
})
