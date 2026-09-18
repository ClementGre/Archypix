// A tag's cover photo by id (feature 34 §6). Thumbnails are stored in raw pixel orientation with
// `orientation` as a column, so a bare `<img>` renders sideways — the detail supplies it.

import {useQuery} from '@tanstack/react-query'
import {getPicture, getPictureUrl} from '@/api/pictures'
import {queryKeys} from '@/lib/constants'
import {cn} from '@/lib/utils'
import {OrientedFillImage} from './OrientedImage'

/** Both queries are ordinary cached ones, and only a tag with an explicit cover mounts this. */
export function OrientedCoverImage({pictureId, alt = '', className}: {
    pictureId: string
    alt?: string
    className?: string
}) {
    const {data: url} = useQuery({
        queryKey: [...queryKeys.picture(pictureId), 'url', 'small'],
        queryFn: () => getPictureUrl(pictureId, 'small'),
        staleTime: 5 * 60_000,
    })
    const {data: detail} = useQuery({
        queryKey: queryKeys.picture(pictureId),
        queryFn: () => getPicture(pictureId),
        staleTime: 5 * 60_000,
    })

    if (!url?.url) return null
    return (
        <span className={cn('relative block overflow-hidden', className)}>
            <OrientedFillImage src={url.url} alt={alt} orientation={detail?.orientation}/>
        </span>
    )
}
