import {type RefObject, useEffect, useState} from 'react'

/**
 * Whether an element is near the viewport. Expanded sections mount their query behind this gate, so
 * a tag with fifty children does not fire fifty requests (feature 35 §5), and release their cards
 * once they scroll far out of view.
 */
export function useInView(
    ref: RefObject<Element | null>,
    opts?: { rootMargin?: string; once?: boolean },
): boolean {
    const {rootMargin = '600px', once = false} = opts ?? {}
    const [inView, setInView] = useState(false)

    useEffect(() => {
        const el = ref.current
        if (!el) return
        const io = new IntersectionObserver(
            ([entry]) => {
                if (!entry) return
                if (entry.isIntersecting) {
                    setInView(true)
                    if (once) io.disconnect()
                } else if (!once) {
                    setInView(false)
                }
            },
            {rootMargin},
        )
        io.observe(el)
        return () => io.disconnect()
    }, [ref, rootMargin, once])

    return inView
}
