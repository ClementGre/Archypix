import {Plus} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger,} from '@/components/ui/dropdown-menu'
import type {ServiceType} from '@/lib/types'

interface NewServiceMenuProps {
    onCreate: (type: ServiceType) => void
    isPending: boolean
}

export function NewServiceMenu({onCreate, isPending}: NewServiceMenuProps) {
    return (
        <DropdownMenu>
            <DropdownMenuTrigger asChild>
                <Button size="sm" disabled={isPending}>
                    <Plus className="mr-1.5 h-4 w-4"/>
                    New service
                </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
                <DropdownMenuItem onSelect={() => onCreate('rule')}>
                    Rule service
                    <span className="ml-2 text-xs text-muted-foreground">predicate → tag</span>
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={() => onCreate('segmentation')}>
                    Segmentation service
                    <span className="ml-2 text-xs text-muted-foreground">date ranges → tag</span>
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={() => onCreate('shared_tag_mapping')}>
                    Shared-tag mapping
                    <span className="ml-2 text-xs text-muted-foreground">share → tag</span>
                </DropdownMenuItem>
            </DropdownMenuContent>
        </DropdownMenu>
    )
}
