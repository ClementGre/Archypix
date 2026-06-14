import {FolderTree, Inbox, Send, Tag} from 'lucide-react'
import {Tabs, TabsContent, TabsList, TabsTrigger} from '@/components/ui/tabs'
import {TagTree} from '@/components/tags/TagTree'
import {IncomingSharesList} from '@/components/shares/IncomingSharesList'
import {OutgoingSharesList} from '@/components/shares/OutgoingSharesList'
import {type LeftPanelTab, useGalleryParams} from '@/hooks/useGalleryParams'

const TABS: { value: LeftPanelTab; label: string; icon: typeof Tag }[] = [
    {value: 'tags', label: 'Tags', icon: Tag},
    {value: 'incoming', label: 'Incoming shares', icon: Inbox},
    {value: 'outgoing', label: 'Outgoing shares', icon: Send},
    {value: 'hierarchies', label: 'Hierarchies', icon: FolderTree},
]

export function LeftPanel() {
    const {params, update} = useGalleryParams()

    return (
        <aside className="w-64 shrink-0 border-r border-border bg-card">
            <Tabs
                value={params.panel}
                onValueChange={(v) => update({panel: v as LeftPanelTab})}
                className="flex h-full flex-col gap-0"
            >
                <TabsList className="grid w-full grid-cols-4 rounded-none border-b border-border bg-transparent p-1">
                    {TABS.map(({value, label, icon: Icon}) => (
                        <TabsTrigger key={value} value={value} title={label} aria-label={label} className="px-0">
                            <Icon className="h-4 w-4"/>
                        </TabsTrigger>
                    ))}
                </TabsList>

                <TabsContent value="tags" className="m-0 min-h-0 flex-1 overflow-hidden">
                    <TagTree/>
                </TabsContent>
                <TabsContent value="incoming" className="m-0 min-h-0 flex-1 overflow-y-auto">
                    <IncomingSharesList/>
                </TabsContent>
                <TabsContent value="outgoing" className="m-0 min-h-0 flex-1 overflow-y-auto">
                    <OutgoingSharesList/>
                </TabsContent>
                <TabsContent value="hierarchies" className="m-0 min-h-0 flex-1 overflow-y-auto">
                    <div className="flex flex-col items-center gap-2 px-4 py-10 text-center text-xs text-muted-foreground">
                        <FolderTree className="h-6 w-6"/>
                        <p className="font-medium text-foreground">Hierarchies</p>
                        <p>Bidirectional WebDAV views of your tag graph. Coming soon.</p>
                    </div>
                </TabsContent>
            </Tabs>
        </aside>
    )
}
