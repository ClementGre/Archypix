// Vidstack-backed player for video/audio pictures (Tier 1: progressive playback of the original
// file straight from S3 via its presigned URL — no transcode). Used by the Lightbox (autoplay) and
// the details panel (audio only; video there shows a poster that opens the Lightbox). The default
// skin's CSS is imported once in main.tsx.

import type {ComponentProps} from 'react'
import type {AudioMimeType, VideoMimeType} from '@vidstack/react'
import {MediaPlayer as VidstackPlayer, MediaProvider} from '@vidstack/react'
import {DefaultAudioLayout, defaultLayoutIcons, DefaultVideoLayout} from '@vidstack/react/player/layouts/default'
import {cn, isAudioMime} from '@/lib/utils'

export function MediaPlayer({src, mime, title, autoPlay = false, aspectRatio, keyTarget, keyShortcuts, className}: {
    src: string
    mime: string | null
    title?: string | null
    autoPlay?: boolean
    /** Video aspect ratio as a `"w/h"` string (e.g. `"16/9"`) — reserves the right shape before the
     *  media loads, avoiding layout shift. Omit for audio or when the dimensions are unknown. */
    aspectRatio?: string
    /** Where Vidstack listens for keyboard shortcuts. `"document"` makes them work without focusing
     *  the player — right for the full-screen Lightbox; leave unset (player-scoped) for inline use. */
    keyTarget?: ComponentProps<typeof VidstackPlayer>['keyTarget']
    /** Override the default keyboard shortcut map (replaces Vidstack's defaults entirely). */
    keyShortcuts?: ComponentProps<typeof VidstackPlayer>['keyShortcuts']
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
            aspectRatio={aspectRatio}
            keyTarget={keyTarget}
            keyShortcuts={keyShortcuts}
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
