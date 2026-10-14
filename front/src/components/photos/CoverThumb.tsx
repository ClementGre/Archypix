// A tag's cover photo by id (feature 34 §6). Thumbnails are stored in raw pixel orientation with
// `orientation` as a column, so a bare `<img>` renders sideways — the detail supplies it.

import {useQuery} from '@tanstack/react-query'
import {getPicture, getPictureUrl} from '@/api/pictures'
import {queryKeys} from '@/lib/constants'
import {cn} from '@/lib/utils'
import {OrientedFillImage} from './OrientedImage'
import {VeilFill} from './SensitiveVeil'
import {useVeil} from '@/hooks/useSensitive'

/** Both queries are ordinary cached ones, and only a tag with an explicit cover mounts this. A
 *  sensitive cover waits for its detail and is never presigned while veiled (feature 38). */
export function OrientedCoverImage({pictureId, alt = '', className}: {
    pictureId: string
    alt?: string
    className?: string
}) {
    const {data: detail} = useQuery({
        queryKey: queryKeys.picture(pictureId),
        queryFn: () => getPicture(pictureId),
        staleTime: 5 * 60_000,
    })
    const veil = useVeil(pictureId, detail?.sensitive)
    const {data: url} = useQuery({
        queryKey: [...queryKeys.picture(pictureId), 'url', 'small'],
        queryFn: () => getPictureUrl(pictureId, 'small'),
        staleTime: 5 * 60_000,
        enabled: !!detail && veil === 'visible',
    })

    if (detail && veil !== 'visible') {
        return (
            <span className={cn('relative block overflow-hidden', className)}>
                <VeilFill veil={veil} blurhash={detail.blurhash} orientation={detail.orientation}
                          width={detail.width} height={detail.height}/>
            </span>
        )
    }
    if (!url?.url) return null
    return (
        <span className={cn('relative block overflow-hidden', className)}>
            <OrientedFillImage src={url.url} alt={alt} orientation={detail?.orientation}/>
        </span>
    )
}
