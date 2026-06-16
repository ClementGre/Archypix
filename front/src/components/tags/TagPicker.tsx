import {type ReactNode, useState} from 'react'
import {Plus, Tag as TagIcon} from 'lucide-react'
import {Popover, PopoverContent, PopoverTrigger} from '@/components/ui/popover'
import {Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList} from '@/components/ui/command'
import {Button} from '@/components/ui/button'
import {useAllTags} from '@/hooks/useTags'
import {TagPath} from '@/lib/utils'

const LABEL_OK = /^[A-Za-z0-9_/]+$/

/** Returns all ancestor wire paths for a given wire path (e.g. `A.B.C` → [`A`, `A.B`]). */
function ancestorWirePaths(wire: string): string[] {
    const parts = wire.split('.')
    const result: string[] = []
    for (let i = 1; i < parts.length; i++) {
        result.push(parts.slice(0, i).join('.'))
    }
    return result
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

  const wireFromInput = q ? TagPath.toWire(q) : ''
  // Protected tags can never be created (the API reserves the prefix).
  const canCreate =
      allowCreate &&
      !!q &&
      LABEL_OK.test(q.replace(/^\/+/, '')) &&
      !!wireFromInput &&
      !TagPath.isProtected(wireFromInput) &&
      !expandedSet.has(wireFromInput)

  const choose = (wire: string) => {
    onSelect(wire)
    setOpen(false)
    setQuery('')
  }

  return (
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
            {trigger ?? (
                <Button variant="outline" size="sm" className="gap-1.5">
                    <Plus className="h-3.5 w-3.5"/>
                    {triggerLabel}
                </Button>
            )}
        </PopoverTrigger>
        <PopoverContent className="w-72 p-0" align="start">
          <Command shouldFilter={false}>
            <CommandInput value={query} onValueChange={setQuery} placeholder={placeholder}/>
            <CommandList>
              {options.length === 0 && !canCreate && <CommandEmpty>No tags found.</CommandEmpty>}
              <CommandGroup>
                {options.map((t) => (
                    <CommandItem key={t} value={t} onSelect={() => choose(t)}>
                      <TagIcon className="mr-2 h-3.5 w-3.5 opacity-60"/>
                      {TagPath.toDisplay(t)}
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
