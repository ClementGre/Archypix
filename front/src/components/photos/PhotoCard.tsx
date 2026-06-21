import {memo, type MouseEvent} from 'react'
import {AlertTriangle, Check, Trash2} from 'lucide-react'
import type {PictureListItem} from '@/lib/types'
import {cn} from '@/lib/utils'
import {countdown} from '@/lib/trash'
import {Blurhash} from './Blurhash'
import {displayDimensions, orientedCoverStyle, OrientedImage} from './OrientedImage'

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
    // Lay the cell out at the picture's *display* orientation (dimensions are
    // transposed for 90°/270° rotations) so the justified grid stays correct.
    const {width: dispW, height: dispH} = displayDimensions(item.width, item.height, item.orientation)
    const ratio = dispW && dispH ? dispW / dispH : 1
    const basis = rowHeight * ratio
    // Rotate the blurhash placeholder the same way as the thumbnail so it lines up behind it.
    const blurhash = orientedCoverStyle(item.orientation, item.width, item.height)

    const trashed = !!item.deleted_at
    const ownerDeleted = !item.owned && !!item.owner_deleted_at

    return (
        <li
            style={{
                flexBasis: `${basis}px`,
                flexGrow: basis,
                aspectRatio: `${dispW ?? 1} / ${dispH ?? 1}`,
            }}
            className={cn(
                'group relative cursor-pointer overflow-hidden rounded-[3px] bg-muted',
                selected && 'ring-2 ring-primary ring-offset-2 ring-offset-background',
                trashed && 'opacity-60',
            )}
            onClick={onSelect}
            onDoubleClick={onOpen}
        >
            {item.blurhash && <Blurhash hash={item.blurhash} className={blurhash.className} style={blurhash.style}/>}

            {item.thumbnail_url && (
                <OrientedImage
                    src={item.thumbnail_url}
                    alt={item.filename ?? ''}
                    orientation={item.orientation}
                    width={item.width}
                    height={item.height}
                    className="opacity-0 transition-opacity duration-200"
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
                <span
                    className={cn(
                        'absolute bottom-1 left-1 flex items-center gap-0.5 rounded px-1 text-[10px] leading-4 text-white',
                        ownerDeleted ? 'bg-destructive/85' : 'bg-black/55',
                    )}
                    title={
                        ownerDeleted
                            ? `Owner deleted this — disappears ${countdown(item.owner_purge_at) || 'soon'}`
                            : undefined
                    }
                >
                    {ownerDeleted && <AlertTriangle className="h-2.5 w-2.5"/>}
                    @{item.owner_username}
                </span>
            )}

            {trashed && (
                <span
                    className="absolute right-1 top-1 flex h-5 w-5 items-center justify-center rounded-full bg-black/55 text-white"
                    title="In trash"
                >
                    <Trash2 className="h-3 w-3"/>
                </span>
            )}
        </li>
    )
})
