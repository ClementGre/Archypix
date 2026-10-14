import {useState} from 'react'
import {AlertTriangle, X} from 'lucide-react'
import {Badge} from '@/components/ui/badge'
import {TagPicker} from '@/components/tags/TagPicker'
import {type ShareInfoEntry, ShareInfoPopover} from '@/components/shares/ShareInfoPopover'
import {useIncomingShares} from '@/hooks/useShares'
import {TagPath} from '@/lib/utils'
import {type ConfigDraft, useReportDraft} from './useReportDraft'
import type {IncomingShareResponse, SharedTagMappingConfig, SharedTagMappingServiceDetail} from '@/lib/types'

const shareHandle = (s: IncomingShareResponse) => `@${s.sender_username}:${s.sender_instance}`

function toEntry(s: IncomingShareResponse): ShareInfoEntry {
    return {
        name: s.name,
        message: s.message,
        status: s.status,
        allowShareBack: s.allow_share_back,
        allowExifEdit: s.allow_exif_edit,
        future: s.future,
        sharedTag: s.shared_tag_path,
        createdAt: s.created_at,
        lastReceivedAt: s.last_announcement_received_at,
        closedAt: s.revoked_at,
    }
}

interface MappingEditorProps {
    service: SharedTagMappingServiceDetail
    onDraft: (d: ConfigDraft) => void
}

/**
 * A shared-tag-mapping service maps one incoming share → a list of local tags.
 * The share is fixed at creation; this edits `assign_tags`, reported to the service editor's Save.
 */
export function MappingEditor({service, onDraft}: MappingEditorProps) {
    const {data: shares} = useIncomingShares()

    const [draft, setDraft] = useState<string[]>(service.assign_tags)
    const serverKey = service.assign_tags.join('|')
    const [syncedKey, setSyncedKey] = useState(serverKey)
    if (serverKey !== syncedKey) {
        setDraft(service.assign_tags)
        setSyncedKey(serverKey)
    }

    const dirty = draft.join('|') !== serverKey
    const share = (shares ?? []).find((s) => s.id === service.incoming_share_id)

    const config: SharedTagMappingConfig = {incoming_share_id: service.incoming_share_id, assign_tags: draft}
    useReportDraft(onDraft, config, dirty)

    return (
        <div className="space-y-3 text-sm">
            <div className="flex items-center gap-2">
                <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Share</span>
                {share ? (
                    <>
                        <span>{share.name || shareHandle(share)}</span>
                        {share.name && <span className="font-mono text-[11px] text-muted-foreground">{shareHandle(share)}</span>}
                        <ShareInfoPopover entries={[toEntry(share)]}/>
                    </>
                ) : (
                    <span className="font-mono text-xs text-muted-foreground">{service.incoming_share_id}</span>
                )}
                {service.is_broken && (
                    <Badge variant="secondary" className="gap-1 border-0 bg-red-500/15 text-red-500">
                        <AlertTriangle className="h-3 w-3"/>
                        broken
                    </Badge>
                )}
            </div>

            {service.is_broken && (
                <p className="rounded-md border border-red-500/30 bg-red-500/5 px-3 py-2 text-xs text-red-500">
                    The incoming share is no longer active. This mapping assigns no tags until the share is restored.
                </p>
            )}

            <div>
                <span className="mb-1.5 block text-xs font-medium uppercase tracking-wide text-muted-foreground">Assign tags</span>
                <div className="flex flex-wrap items-center gap-1.5">
                    {draft.map((tag) => (
                        <Badge key={tag} variant="secondary" className="gap-1 pr-1">
                            {TagPath.toDisplay(tag)}
                            <button onClick={() => setDraft(draft.filter((t) => t !== tag))}
                                    className="ml-0.5 rounded-full p-0.5 hover:bg-foreground/10">
                                <X className="h-2.5 w-2.5"/>
                            </button>
                        </Badge>
                    ))}
                    <TagPicker
                        onSelect={(wire) => !draft.includes(wire) && setDraft([...draft, wire])}
                        excludePaths={draft}
                        allowCreate
                        triggerLabel="Add tag"
                    />
                </div>
            </div>

        </div>
    )
}
