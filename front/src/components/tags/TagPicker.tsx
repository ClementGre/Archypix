import {type ReactNode, useState} from 'react'
import {AlertTriangle, ChevronRight, Plus, Tag as TagIcon} from 'lucide-react'
import {Popover, PopoverContent, PopoverTrigger} from '@/components/ui/popover'
import {Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList} from '@/components/ui/command'
import {Button} from '@/components/ui/button'
import {useAllTags} from '@/hooks/useTags'
import {TagPath} from '@/lib/utils'

const LABEL_OK = /^[A-Za-z0-9_/]+$/
const VALID_CHAR = /[A-Za-z0-9_/]/

/** Returns all ancestor wire paths for a given wire path (e.g. `A.B.C` → [`A`, `A.B`]). */
function ancestorWirePaths(wire: string): string[] {
    const parts = wire.split('.')
    const result: string[] = []
    for (let i = 1; i < parts.length; i++) {
        result.push(parts.slice(0, i).join('.'))
    }
    return result
}

interface Sanitized {
    /** The input with auto-fixable characters replaced (kept in the field). */
    clean: string
    /** Human-readable replacement notes (orange warnings). */
    replaced: string[]
}

/**
 * Auto-fix common typos as the user types display-form tag input:
 *  - strip accents (é → e),
 *  - spaces / `-` → `_`,
 *  - `.` / `\` → `/` (the display-form delimiter).
 * Characters that can't be mapped are kept verbatim so the caller can flag them in red.
 */
function sanitizeTagInput(raw: string): Sanitized {
    const replaced: string[] = []

    const deaccented = raw.normalize('NFD').replace(/[\u0300-\u036f]/g, '')
    if (deaccented !== raw) replaced.push('Removed accents')

    let s = deaccented
    if (/[ \-]/.test(s)) {
        s = s.replace(/[ \-]+/g, '_')
        replaced.push('Invalid character changed to "_"')
    }
    if (/[.\\]/.test(s)) {
        s = s.replace(/[.\\]+/g, '/')
        replaced.push('Invalid character changed to "/"')
    }
    return {clean: s, replaced}
}

/** Distinct characters still invalid after sanitization. */
function invalidChars(s: string): string[] {
    const set = new Set<string>()
    for (const ch of s) if (!VALID_CHAR.test(ch)) set.add(ch)
    return [...set]
}

interface TagPickerProps {
  /** Called with the chosen/created tag in WIRE form (e.g. `Photos.Travel`). */
  onSelect: (wirePath: string) => void
  /** Tags to hide from the list (e.g. already assigned). */
  excludePaths?: string[]
  allowCreate?: boolean
  /**
   * Whether protected tags (`SharedToMe.*`) may be listed/selected. Off for
   * manual tagging and share-mappings; on for sharing and service gates.
   */
  allowProtected?: boolean
  triggerLabel?: string
  placeholder?: string
    /** Custom trigger element (rendered `asChild`); overrides the default button. */
    trigger?: ReactNode
}

/** Autocomplete over the user's existing tags, with optional create-new. */
export function TagPicker({
                            onSelect,
                            excludePaths = [],
                            allowCreate = true,
                            allowProtected = false,
                            triggerLabel = 'Add tag',
                            placeholder = 'Search or create tag…',
                              trigger,
                          }: TagPickerProps) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [replaced, setReplaced] = useState<string[]>([])
  // The cmdk-highlighted item value (a wire path, or a `__create__…` token).
  const [active, setActive] = useState('')
  const {data: tags} = useAllTags()

    const allTags = tags ?? []

    // Ancestors virtually covered by already-assigned tags must also be excluded.
    const excludeSet = new Set(excludePaths)
    for (const exc of excludePaths) {
        for (const anc of ancestorWirePaths(exc)) {
            excludeSet.add(anc)
        }
    }

    // Expand the suggestion universe to include ancestor paths of every known tag.
    const expandedSet = new Set<string>(allTags)
    for (const t of allTags) {
        for (const anc of ancestorWirePaths(t)) {
            expandedSet.add(anc)
        }
    }

    const all = Array.from(expandedSet)
        .filter((t) => !excludeSet.has(t) && (allowProtected || !TagPath.isProtected(t)))
        .sort()

  const q = query.trim()
  const options = q ? all.filter((t) => TagPath.toDisplay(t).toLowerCase().includes(q.toLowerCase())) : all

  const bad = invalidChars(q)
  const wireFromInput = q ? TagPath.toWire(q) : ''
  const wouldBeNewProtected = !!wireFromInput && TagPath.isProtected(wireFromInput) && !expandedSet.has(wireFromInput)
  // Protected tags can never be created (the API reserves the prefix).
  const canCreate =
      allowCreate &&
      !!q &&
      bad.length === 0 &&
      LABEL_OK.test(q.replace(/^\/+/, '')) &&
      !!wireFromInput &&
      !TagPath.isProtected(wireFromInput) &&
      !expandedSet.has(wireFromInput)

  const onInput = (raw: string) => {
    const s = sanitizeTagInput(raw)
    setQuery(s.clean)
    setReplaced(s.replaced)
  }

  // Fill the field with `<tag>/` so the user can append a child without retyping the prefix
  // (e.g. autocomplete `/Event` then type `Birthday` to create `/Event/Birthday`).
  const autocompleteInto = (wire: string) => {
    setQuery(TagPath.toDisplay(wire) + '/')
    setReplaced([])
  }

  const choose = (wire: string) => {
    onSelect(wire)
    setOpen(false)
    setQuery('')
    setReplaced([])
  }

  return (
      <Popover open={open} onOpenChange={(o) => {
          setOpen(o)
          if (!o) {
              setQuery('')
              setReplaced([])
          }
      }}>
        <PopoverTrigger asChild>
            {trigger ?? (
                <Button variant="outline" size="sm" className="gap-1.5">
                    <Plus className="h-3.5 w-3.5"/>
                    {triggerLabel}
                </Button>
            )}
        </PopoverTrigger>
        <PopoverContent className="w-72 p-0" align="start">
          <Command shouldFilter={false} value={active} onValueChange={setActive}>
            <CommandInput
                value={query}
                onValueChange={onInput}
                placeholder={placeholder}
                onKeyDown={(e) => {
                    // Tab autocompletes the highlighted existing tag into the field as a prefix.
                    if (e.key === 'Tab' && !e.shiftKey && active && !active.startsWith('__create__') && expandedSet.has(active)) {
                        e.preventDefault()
                        autocompleteInto(active)
                    }
                }}
            />

            {/* Inline validation: orange auto-fixes, red blockers. */}
            {(replaced.length > 0 || bad.length > 0 || wouldBeNewProtected) && (
                <div className="space-y-1 border-b px-2 py-1.5 text-[11px]">
                    {replaced.length > 0 && (
                        <p className="flex items-start gap-1 text-amber-500">
                            <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0"/>
                            <span>{replaced.join(' · ')}</span>
                        </p>
                    )}
                    {wouldBeNewProtected && (
                        <p className="flex items-start gap-1 text-destructive">
                            <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0"/>
                            <span>“SharedToMe” is a reserved prefix and can’t be used.</span>
                        </p>
                    )}
                    {bad.length > 0 && (
                        <p className="flex items-start gap-1 text-destructive">
                            <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0"/>
                            <span>
                                Not allowed: {bad.map((c) => `“${c}”`).join(' ')}. Use letters, numbers, “_” or “/”.
                            </span>
                        </p>
                    )}
                </div>
            )}

            <CommandList>
              {options.length === 0 && !canCreate && <CommandEmpty>No tags found.</CommandEmpty>}
              <CommandGroup>
                {options.map((t) => (
                    <CommandItem key={t} value={t} onSelect={() => choose(t)} className="group/item">
                      <TagIcon className="mr-2 h-3.5 w-3.5 opacity-60"/>
                      <span className="min-w-0 flex-1 truncate">{TagPath.toDisplay(t)}</span>
                      <button
                          type="button"
                          onMouseDown={(e) => {
                              e.preventDefault()
                              e.stopPropagation()
                          }}
                          onClick={(e) => {
                              e.stopPropagation()
                              autocompleteInto(t)
                          }}
                          title="Use as prefix for a new sub-tag"
                          className="ml-2 shrink-0 rounded p-0.5 text-muted-foreground opacity-50 hover:bg-accent hover:text-foreground"
                      >
                          <ChevronRight className="h-3.5 w-3.5"/>
                      </button>
                    </CommandItem>
                ))}
                {canCreate && (
                    <CommandItem value={`__create__${wireFromInput}`} onSelect={() => choose(wireFromInput)}>
                      <Plus className="mr-2 h-3.5 w-3.5"/>
                      Create “{TagPath.toDisplay(wireFromInput)}”
                    </CommandItem>
                )}
              </CommandGroup>
            </CommandList>
          </Command>
        </PopoverContent>
      </Popover>
  )
}
