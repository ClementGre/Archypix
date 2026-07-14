import {useThemeStore} from '@/stores/theme'
import {cn} from '@/lib/utils'

/**
 * Archypix brand wordmark. Swaps between the light- and dark-theme artwork (they carry the same
 * shape, only the green shades are tuned for the background) from the theme store, and uses a
 * root-absolute `src` so it still resolves on nested routes — notably the public share page at
 * `/s/:globalDomain/:username/:token`, where a relative path would 404.
 */
export function Logo({className}: { className?: string }) {
    const theme = useThemeStore((s) => s.theme)
    return (
        <img
            src={theme === 'light' ? '/logo-light.svg' : '/logo-dark.svg'}
            alt="Archypix"
            className={cn('h-5 w-auto', className)}
        />
    )
}
