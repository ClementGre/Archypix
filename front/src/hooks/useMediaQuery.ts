import {useEffect, useState} from 'react'

/** Reactively track a CSS media query. */
export function useMediaQuery(query: string): boolean {
    const [matches, setMatches] = useState(() =>
        typeof window !== 'undefined' ? window.matchMedia(query).matches : false,
    )

    useEffect(() => {
        const mql = window.matchMedia(query)
        const onChange = () => setMatches(mql.matches)
        onChange()
        mql.addEventListener('change', onChange)
        return () => mql.removeEventListener('change', onChange)
    }, [query])

    return matches
}

/** True on viewports narrower than Tailwind's `md` breakpoint (768px). */
export function useIsMobile(): boolean {
    return useMediaQuery('(max-width: 767px)')
}
