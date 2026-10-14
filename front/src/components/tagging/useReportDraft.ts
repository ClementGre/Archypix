import {useEffect} from 'react'
import type {ServiceConfig} from '@/lib/types'

/** A type editor's config draft, owned by the service editor's single Save. */
export interface ConfigDraft {
    config: ServiceConfig
    dirty: boolean
}

/** Report a type editor's draft up to the service editor whenever it changes. */
export function useReportDraft(onDraft: (d: ConfigDraft) => void, config: ServiceConfig, dirty: boolean) {
    const key = JSON.stringify(config)
    useEffect(() => {
        onDraft({config: JSON.parse(key) as ServiceConfig, dirty})
    }, [key, dirty, onDraft])
}
