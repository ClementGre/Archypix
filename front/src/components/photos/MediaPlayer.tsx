// Vidstack-backed player for video/audio pictures (Tier 1: progressive playback of the original
// file straight from S3 via its presigned URL — no transcode). Used by the Lightbox (autoplay) and
// the details panel (audio only; video there shows a poster that opens the Lightbox). The default
// skin's CSS is imported once in main.tsx.

import type {AudioMimeType, VideoMimeType} from '@vidstack/react'
import {MediaPlayer as VidstackPlayer, MediaProvider} from '@vidstack/react'
import {DefaultAudioLayout, defaultLayoutIcons, DefaultVideoLayout} from '@vidstack/react/player/layouts/default'
import {cn, isAudioMime} from '@/lib/utils'

export function MediaPlayer({src, mime, title, autoPlay = false, className}: {
    src: string
    mime: string | null
    title?: string | null
    autoPlay?: boolean
    className?: string
}) {
    const audio = isAudioMime(mime)
    return (
        <VidstackPlayer
            className={cn('w-full overflow-hidden rounded-md', className)}
            // Pass the mime as a type hint — presigned URLs carry no clean extension to infer from.
            // The browser still validates against the response Content-Type; the cast just satisfies
            // Vidstack's literal mime union (our value is always a real audio/* or video/* type).
            src={mime ? {src, type: mime as VideoMimeType | AudioMimeType} : src}
            viewType={audio ? 'audio' : 'video'}
            title={title ?? undefined}
            autoPlay={autoPlay}
            playsInline
            // Keep clicks off the Lightbox backdrop (which closes on click).
            onClick={(e) => e.stopPropagation()}
        >
            <MediaProvider/>
            {audio
                ? <DefaultAudioLayout icons={defaultLayoutIcons}/>
                : <DefaultVideoLayout icons={defaultLayoutIcons}/>}
        </VidstackPlayer>
    )
}
