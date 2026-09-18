// Cover photo for a tag (feature 34 §6). Setting one lives where the photos are — pick the picture
// in the grid and use its `⋯` → *Set as thumbnail* — so this is a preview + clear, not a second
// picture browser nested inside the tag dialog.

import {ImageIcon, X} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {Label} from '@/components/ui/label'
import {OrientedCoverImage} from '@/components/photos/CoverThumb'

export function CoverPicker({value, onChange}: {
    value: string | null
    onChange: (pictureId: string | null) => void
}) {
    return (
        <div className="space-y-1.5">
            <Label>Thumbnail</Label>
            <div className="flex items-start gap-2">
                {value ? (
                    <OrientedCoverImage pictureId={value} className="h-12 w-16 shrink-0 rounded border"/>
                ) : (
                    <div className="flex h-12 w-16 shrink-0 items-center justify-center rounded border border-dashed text-muted-foreground">
                        <ImageIcon className="h-4 w-4"/>
                    </div>
                )}
                <p className="min-w-0 flex-1 text-[11px] text-muted-foreground">
                    Select a photo under this tag, then use its <span className="font-medium">⋯</span> menu →{' '}
                    <span className="font-medium">Set as thumbnail</span>. Without one, the first photo loaded
                    stands in.
                </p>
                {value && (
                    <Button variant="ghost" size="sm" className="shrink-0 gap-1 text-xs" onClick={() => onChange(null)}>
                        <X className="h-3 w-3"/>
                        Clear
                    </Button>
                )}
            </div>
        </div>
    )
}
