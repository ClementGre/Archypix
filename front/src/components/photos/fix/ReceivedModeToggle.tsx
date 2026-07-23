// Received-picture apply mode (feature 30 §9): a private local override, or a propose-to-owner edit
// (only when the share grants it). Shared by the GPS and Date fix panels.

import type {FixReceivedMode} from '@/hooks/useFixApply'
import {cn} from '@/lib/utils'

export function ReceivedModeToggle({value, onChange, allowPropose}: {
    value: FixReceivedMode
    onChange: (v: FixReceivedMode) => void
    allowPropose: boolean
}) {
    return (
        <div className="flex items-center justify-center gap-0.5 rounded-md border border-border p-0.5 text-xs">
            {(['local', 'propose'] as const).map((m) => (
                <button
                    key={m}
                    type="button"
                    disabled={m === 'propose' && !allowPropose}
                    onClick={() => onChange(m)}
                    title={m === 'propose' && !allowPropose ? "This share doesn't allow proposing to the owner" : undefined}
                    className={cn(
                        'flex-1 rounded px-1.5 py-0.5 transition-colors disabled:opacity-40',
                        value === m ? 'bg-primary/15 text-primary' : 'text-muted-foreground hover:text-foreground',
                    )}
                >
                    {m === 'local' ? 'Local override' : 'Propose to owner'}
                </button>
            ))}
        </div>
    )
}
