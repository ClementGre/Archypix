import {type ReactNode, useState} from 'react'
import {AlertTriangle, ChevronRight, Plus, Settings2, Tag as TagIcon} from 'lucide-react'
import {Popover, PopoverContent, PopoverTrigger} from '@/components/ui/popover'
import {Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList} from '@/components/ui/command'
import {Button} from '@/components/ui/button'
import {NewTagDialog} from '@/components/tags/NewTagDialog'
import {useTagTree, useWriteTagMeta} from '@/hooks/useTags'
import {displayPath} from '@/lib/tagTree'
import {TagPath} from '@/lib/utils'

/** Returns all ancestor wire paths for a given wire path (e.g. `A.B.C` → [`A`, `A.B`]). */
function ancestorWirePaths(wire: string): string[] {
    const parts = wire.split('.')
    const result: string[] = []
    for (let i = 1; i < parts.length; i++) {
        result.push(parts.slice(0, i).join('.'))
    }
    return result
}

/** What the typed text would create: the slugified path plus the leaf as typed, which becomes the
 *  display name (feature 34 §5). The field accepts anything — only the slug is the identity. */
interface TagDraft {
    path: string
    parent: string | null
    /** The leaf segment verbatim, e.g. `Vietnam 🇻🇳 2024`. */
    leaf: string
}

function draftFor(input: string): TagDraft | null {
    const segments = input.split('/').map((s) => s.trim()).filter(Boolean)
    if (!segments.length) return null
    const path = segments.map(TagPath.slugify).join('.')
    return {path, parent: TagPath.parent(path), leaf: segments[segments.length - 1]}
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
  // The cmdk-highlighted item value (a wire path, or a `__create__…` token).
  const [active, setActive] = useState('')
  /** The draft handed to the create dialog, snapshotted when the popover closes behind it. */
  const [customize, setCustomize] = useState<TagDraft | null>(null)
  const {items, metaByPath} = useTagTree()
  const write = useWriteTagMeta()

    const allTags = items.map((i) => i.path).filter(Boolean)
    /** Named path over raw path, and **search matches both** (feature 34 §5). */
    const namedOf = (wire: string) => displayPath(wire, metaByPath)

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
  // Matches the display name, the display path and the raw wire path — the field takes free text,
  // so a pasted `Era.2026.Vietnam` has to find its tag as readily as a typed `/Era/2026`.
  const options = q
      ? all.filter((t) => {
          const needle = q.toLowerCase()
          return t.toLowerCase().includes(needle)
              || TagPath.toDisplay(t).toLowerCase().includes(needle)
              || namedOf(t).toLowerCase().includes(needle)
      })
      : all

  const typed = draftFor(q)
  const draft = typed && !expandedSet.has(typed.path) ? typed : null
  // Protected tags can never be created (the API reserves the prefix).
  const wouldBeNewProtected = !!draft && TagPath.isProtected(draft.path)
  const canCreate = allowCreate && !!draft && !wouldBeNewProtected

  // Fill the field with `<tag>/` so the user can append a child without retyping the prefix
  // (e.g. autocomplete `/Event` then type `Birthday` to create `/Event/Birthday`).
  const autocompleteInto = (wire: string) => setQuery(TagPath.toDisplay(wire) + '/')

  const choose = (wire: string) => {
    onSelect(wire)
    setOpen(false)
    setQuery('')
  }

  /** Create straight from the typed text: the slug is the path, what was typed is the name (§5). */
  const createTyped = () => {
      if (!draft) return
      if (draft.leaf !== TagPath.leaf(draft.path)) write({tag_path: draft.path, display_name: draft.leaf})
      choose(draft.path)
  }

  return (
      <>
      <Popover open={open} onOpenChange={(o) => {
          setOpen(o)
          if (!o) setQuery('')
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
                onValueChange={setQuery}
                placeholder={placeholder}
                onKeyDown={(e) => {
                    // Tab autocompletes the highlighted existing tag into the field as a prefix.
                    if (e.key === 'Tab' && !e.shiftKey && active && !active.startsWith('__create__') && expandedSet.has(active)) {
                        e.preventDefault()
                        autocompleteInto(active)
                    }
                }}
            />

            {wouldBeNewProtected && (
                <p className="flex items-start gap-1 border-b px-2 py-1.5 text-[11px] text-destructive">
                    <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0"/>
                    <span>“SharedToMe” is a reserved prefix and can’t be used.</span>
                </p>
            )}

            <CommandList>
              {options.length === 0 && !canCreate && <CommandEmpty>No tags found.</CommandEmpty>}
              <CommandGroup>
                {options.map((t) => (
                    <CommandItem key={t} value={t} onSelect={() => choose(t)} className="group/item">
                      <TagIcon className="mr-2 h-3.5 w-3.5 shrink-0 opacity-60"/>
                      {/* Named path over raw path — a display name renames a segment, it never hides
                          where the tag lives, and this surface writes the ltree path (§5). */}
                      <span className="flex min-w-0 flex-1 flex-col">
                          <span className="truncate">{namedOf(t)}</span>
                          {namedOf(t) !== TagPath.toDisplay(t) && (
                              <span className="truncate text-[11px] text-muted-foreground">{TagPath.toDisplay(t)}</span>
                          )}
                      </span>
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
                {canCreate && draft && (
                    <CommandItem value={`__create__${draft.path}`} onSelect={createTyped}>
                      <Plus className="mr-2 h-3.5 w-3.5 shrink-0"/>
                      <span className="flex min-w-0 flex-1 flex-col">
                          <span className="truncate">Create “{draft.leaf}”</span>
                          <span className="truncate font-mono text-[11px] text-muted-foreground">
                              {TagPath.toDisplay(draft.path)}
                          </span>
                      </span>
                      {/* Configure the new tag before it is used, rather than create-then-edit. */}
                      <button
                          type="button"
                          onMouseDown={(e) => {
                              e.preventDefault()
                              e.stopPropagation()
                          }}
                          onClick={(e) => {
                              e.stopPropagation()
                              setCustomize(draft)
                              setOpen(false)
                          }}
                          className="ml-2 flex shrink-0 items-center gap-1 rounded px-1 py-0.5 text-[11px] text-muted-foreground hover:bg-accent hover:text-foreground"
                      >
                          <Settings2 className="h-3.5 w-3.5"/>
                          Customize
                      </button>
                    </CommandItem>
                )}
              </CommandGroup>
            </CommandList>
          </Command>
        </PopoverContent>
      </Popover>

      {customize && (
          <NewTagDialog
              parentPath={customize.parent}
              initialName={customize.leaf}
              // Photos follow right after, so the tag does not need to survive as an empty one.
              emptyByDefault={false}
              open
              onOpenChange={(o) => !o && setCustomize(null)}
              onCreated={onSelect}
          />
      )}
      </>
  )
}
