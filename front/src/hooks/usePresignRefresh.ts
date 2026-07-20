import {useCallback, useRef} from 'react'

/**
 * An `<img>` `onError` handler that treats an expired / `403` presigned URL as a refresh signal
 * (feature 28 §10): it re-requests a fresh URL via `refresh` rather than leaving a broken image.
 * Guards against loops by refreshing at most once per failing `src` — a genuinely broken image
 * (a second failure on the fresh URL) is left as-is.
 */
export function usePresignRefresh(refresh: () => void): React.ReactEventHandler<HTMLImageElement> {
    const triedFor = useRef<string | null>(null)
    return useCallback(
        (e) => {
            const src = e.currentTarget.currentSrc || e.currentTarget.src
            if (!src || triedFor.current === src) return
            triedFor.current = src
            refresh()
        },
        [refresh],
    )
}
