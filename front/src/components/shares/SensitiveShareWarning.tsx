import {useQuery} from '@tanstack/react-query'
import {AlertTriangle} from 'lucide-react'
import {listPictures} from '@/api/pictures'

/** Warn when a share's tag covers sensitive pictures (feature 38 §7) — sharing is never blocked. */
export function SensitiveShareWarning({tag, audience}: { tag: string; audience: 'recipients' | 'visitors' }) {
    const {data} = useQuery({
        queryKey: ['pictures', 'sensitive-count', tag],
        queryFn: () => listPictures({page: 1, page_size: 1, include_tags: tag, sensitive: 'present'}),
        enabled: !!tag,
        staleTime: 60_000,
    })
    const n = data?.total ?? 0
    if (!tag || n === 0) return null
    return (
        <p className="flex items-start gap-1.5 text-[11px] text-amber-600 dark:text-amber-400">
            <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0"/>
            <span>
                {n} sensitive picture{n === 1 ? '' : 's'} in this tag —{' '}
                {audience === 'recipients' ? 'recipients will see them normally.' : 'visitors will see them blurred until revealed.'}
            </span>
        </p>
    )
}
