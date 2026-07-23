import {useMemo} from 'react'
import {useIncomingShares} from '@/hooks/useShares'

/**
 * Whether the recipient may **propose** EXIF edits for a picture to its owner (feature 10). Owned
 * pictures always write through (returns `true`); a received picture is editable-to-owner only when
 * a live incoming share from that owner grants `allow_exif_edit`. Mirrors `SelectionPanel`'s match.
 */
export function useAllowExifEdit(
    picture: { owner_username: string | null; owner_instance_domain?: string | null } | null,
): boolean {
    const {data: incoming} = useIncomingShares()
    return useMemo(() => {
        if (!picture || picture.owner_username == null) return true
        const live = (incoming ?? []).filter((s) => s.status === 'active' || s.status === 'pending')
        const share = live.find(
            (s) => s.sender_username === picture.owner_username && s.sender_instance === picture.owner_instance_domain,
        )
        return !!share?.allow_exif_edit
    }, [incoming, picture])
}
