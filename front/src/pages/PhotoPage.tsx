import {useParams} from 'react-router-dom'
import {PagePlaceholder} from '@/components/layout/PagePlaceholder'

export default function PhotoPage() {
    const {id} = useParams()
    return (
        <PagePlaceholder
            title="Photo"
            description="Full-size view with EXIF, version history, tag provenance, and which shares cover this picture."
        >
            Detail view for picture <code className="text-foreground">{id}</code> — coming soon.
        </PagePlaceholder>
    )
}
