import {useParams} from 'react-router-dom'
import {PagePlaceholder} from '@/components/layout/PagePlaceholder'

export default function ServiceEditorPage() {
    const {id} = useParams()
    return (
        <PagePlaceholder
            title="Service editor"
            description="Configure a tagging service's rules, date-range segments or share mappings, plus its requires/excludes gates."
        >
            Editor for service <code className="text-foreground">{id}</code> — coming soon.
        </PagePlaceholder>
    )
}
