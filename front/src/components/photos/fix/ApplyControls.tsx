// Apply controls for the single-picture fix panels (feature 30 §8). A split button whose main action
// is the last-used one (Apply, or Apply & next) with a dropdown to pick either, plus a separate Skip
// that advances to the next picture without writing.

import {Check, ChevronDown, Loader2, SkipForward} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger} from '@/components/ui/dropdown-menu'
import {useFixPrefs} from '@/stores/fixPrefs'

export function ApplyControls({onApply, onApplyNext, onSkip, disabled, saving}: {
    onApply: () => void
    onApplyNext: () => void
    onSkip: () => void
    disabled?: boolean
    saving?: boolean
}) {
    const mode = useFixPrefs((s) => s.applyMode)
    const setMode = useFixPrefs((s) => s.setApplyMode)
    const label = mode === 'applyNext' ? 'Apply & next' : 'Apply'
    const run = () => (mode === 'applyNext' ? onApplyNext() : onApply())

    return (
        <div className="flex gap-1.5">
            <div className="flex flex-1">
                <Button size="sm" className="flex-1 gap-1.5 rounded-r-none" disabled={disabled || saving} onClick={run}>
                    {saving ? <Loader2 className="h-4 w-4 animate-spin"/> : <Check className="h-4 w-4"/>} {label}
                </Button>
                <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                        <Button size="sm" className="rounded-l-none border-l border-primary-foreground/25 px-1.5" disabled={disabled || saving}
                                aria-label="Apply options">
                            <ChevronDown className="h-4 w-4"/>
                        </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end">
                        <DropdownMenuItem onSelect={() => {
                            setMode('apply');
                            onApply()
                        }}>Apply</DropdownMenuItem>
                        <DropdownMenuItem onSelect={() => {
                            setMode('applyNext');
                            onApplyNext()
                        }}>Apply &amp; next</DropdownMenuItem>
                    </DropdownMenuContent>
                </DropdownMenu>
            </div>
            <Button size="sm" variant="outline" className="gap-1.5" disabled={saving} onClick={onSkip} title="Leave this one and jump to the next">
                <SkipForward className="h-4 w-4"/> Skip
            </Button>
        </div>
    )
}
