import {useCallback, useState} from 'react'

/** A boolean that persists to localStorage — used for foldable section state. */
export function usePersistentBool(key: string, defaultValue: boolean): [boolean, (value?: boolean) => void] {
    const storageKey = `archypix_ui_${key}`
    const [value, setValue] = useState<boolean>(() => {
        const raw = localStorage.getItem(storageKey)
        return raw === null ? defaultValue : raw === '1'
    })

    const set = useCallback(
        (next?: boolean) => {
            setValue((prev) => {
                const resolved = next === undefined ? !prev : next
                localStorage.setItem(storageKey, resolved ? '1' : '0')
                return resolved
            })
        },
        [storageKey],
    )

    return [value, set]
}
