// One tag's stream (feature 35 §3, §5): its direct photos partitioned into grouping sections, with
// its child tags placed as blocks. Expanding a block recurses with **that child's own** remembered
// view mode, grouping and placement — which is what makes the rule work identically at the root, at
// `Era`, and at `Era/2026/Vietnam` with no special cases.

import {type ReactNode, useEffect, useMemo, useRef} from 'react'
import {AlertCircle, ChevronRight, Hash, ImageOff, Loader2} from 'lucide-react'
import {apiErrorMessage} from '@/api/client'
import {usePictures} from '@/hooks/usePictures'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {useInView} from '@/hooks/useInView'
import {useTagTree} from '@/hooks/useTags'
import {
    type BucketContext,
    dateBucketFor,
    type GroupingKind,
    isGrouped,
    mergeSections,
    resolveGrouping,
    type Section,
    sectionize,
} from '@/lib/grouping'
import {partitionUndated, placingDate, resolvePlacement, resolveViewMode} from '@/lib/timeline'
import {countsFor, effectiveDate, type TagNode, type TrashView} from '@/lib/tagTree'
import {cn} from '@/lib/utils'
import {useTimelineExpansion} from '@/stores/timelineExpansion'
import {useUIStore} from '@/stores/ui'
import type {PictureFilters, PictureListItem, TagViewMode} from '@/lib/types'
import {useRegisterSection} from './GroupedGridContext'
import {PhotoCards, useFixTargetDetail, useGridVariant} from './PhotoCards'
import {formatRange, OpenTagButton, SubtagBlock} from './SubtagBlock'

/** Blocks placed once before the first section (`top`); `-1` keeps them ahead of it in order (§8). */
const TOP_BLOCKS = -1

/** Every sticky header is this tall, so a nested one can offset itself by its ancestors' heights. */
const HEADER_H = 28

/** Sticky headers stack outermost-on-top; deeper ones slide *under* their ancestors. */
const headerZ = (stickyTop: number) => Math.max(1, 30 - stickyTop / HEADER_H)

/** The padding a header sits inside and has to bleed back out of: the grid container's `p-3` at the
 *  top level, an expanded card's `p-1.5` for the stream nested in it. A header inset by its
 *  container's padding reads as a floating bar with the page showing down either side of it. */
const PAD_ROOT = 12
const PAD_CARD = 6

/** Full-bleed background, content still aligned with the rows below it. */
const bleed = (pad: number) => ({marginInline: -pad, paddingInline: pad})

/** What every level of the recursion carries down unchanged. */
export interface StreamContext {
    trash: TrashView
    bucketCtx: BucketContext
    /** Cross-cutting `inc`/`exc` + scope, layered onto every query in the view (§7). */
    base: PictureFilters
    /** Counts come from the unfiltered tag payload, so they are hidden under `inc`/`exc` (§5). */
    hideCounts: boolean
    onDropOnTag: (node: TagNode) => void
}

export interface TagStreamProps extends StreamContext {
    /** `''` at the root, where "direct photos" are the untagged ones and children are top-level tags. */
    path: string
    /** The tree node, or `null` at the root. */
    node: TagNode | null
    /** Position in the flattened visible order (§8); each level appends its index. */
    order: number[]
    /** Fix mode pins the top-level stream flat; nested streams always read their own row (§9). */
    modeOverride?: TagViewMode
    groupingOverride?: GroupingKind
    /** Nested streams mount their query only once visible; the top-level one is always live. */
    gated?: boolean
    /** Where this level's sticky headers pin, below the headers of every level above it. */
    stickyTop?: number
    /** The horizontal padding this level sits inside, for its headers to bleed out of. */
    pad?: number
}

export function TagStream({path, node, order, modeOverride, groupingOverride, gated, stickyTop = 0, pad = PAD_ROOT, ...ctx}: TagStreamProps) {
    const {trash, bucketCtx, base, hideCounts} = ctx
    const {params} = useGalleryParams()
    const {tree, metaByPath} = useTagTree(trash)
    const variant = useGridVariant()
    const {geoRef} = useFixTargetDetail()
    const rowHeight = useUIStore((s) => s.rowHeight)

    const meta = metaByPath.get(path) ?? null
    const mode = modeOverride ?? resolveViewMode(meta)
    const grouping = groupingOverride ?? resolveGrouping(meta, params.sort)
    const placement = resolvePlacement(meta, path, params.sort)

    const gateRef = useRef<HTMLDivElement>(null)
    // Once opened a section stays mounted: re-firing on every scroll-by would defeat the cache.
    const opened = useInView(gateRef, {once: true})
    const enabled = !gated || opened

    // `subtag` and `direct` both render T's own photos; only `all` widens to the subtree.
    const filters: PictureFilters = useMemo(
        () => ({
            ...base,
            tag: mode === 'all' ? path || null : null,
            exact: mode === 'all' || path === '' ? null : path,
            untagged: mode !== 'all' && path === '',
        }),
        [base, mode, path],
    )

    const q = usePictures(filters, {enabled, variant, geoRef})
    const items = useMemo(() => {
        const flat = q.data?.pages.flatMap((p) => p.items) ?? []
        const seen = new Set<string>()
        return flat.filter((it) => (seen.has(it.id) ? false : (seen.add(it.id), true)))
    }, [q.data])

    const children = useMemo(
        () => (mode === 'subtag' ? (node ? node.children : tree) : []),
        [mode, node, tree],
    )
    const {dated, undated} = useMemo(
        () => partitionUndated(children, params.order, trash),
        [children, params.order, trash],
    )

    // Under `in_sections` a child sits in the section its sort-leading endpoint falls into; a child
    // dated where T has no photos of its own still needs a section, hence the merge.
    const inSections = placement === 'in_sections' && isGrouped(grouping) && dated.length > 0
    const bucketKeyOf = (child: TagNode) =>
        dateBucketFor(placingDate(child, params.order, trash), grouping, bucketCtx)

    const sections: Section<PictureListItem>[] = useMemo(() => {
        const photoSections = sectionize(items, params.sort, grouping, bucketCtx)
        if (!inSections) return photoSections
        const buckets = dated.map((c) =>
            dateBucketFor(placingDate(c, params.order, trash), grouping, bucketCtx),
        )
        return mergeSections(photoSections, buckets, params.order)
    }, [items, params.sort, params.order, grouping, bucketCtx, inSections, dated, trash])

    // Nothing to interleave with ⇒ children expand by default; the visibility gate keeps that cheap
    // (§5). Root counts are meaningless (34 §16), so the root never auto-expands.
    const autoExpand = !!node && countsFor(node, trash).exact_count === 0

    const topBlocks = inSections ? undated : [...dated, ...undated]
    const inner: StreamContext = ctx

    const body = (
        <>
            {topBlocks.length > 0 && (
                <Blocks blocks={topBlocks} order={[...order, TOP_BLOCKS]} autoExpand={autoExpand}
                        stickyTop={stickyTop} ctx={inner}/>
            )}
            {sections.map((section, i) => (
                <GroupSection
                    key={section.key || 'all'}
                    section={section}
                    showHeader={isGrouped(grouping)}
                    settled={!q.hasNextPage && !q.isFetching}
                    order={[...order, i]}
                    stickyTop={stickyTop}
                    pad={pad}
                    blocks={inSections ? dated.filter((c) => bucketKeyOf(c).key === section.key) : []}
                    sectionKey={`${path}::${section.key}`}
                    fetchNextPage={q.fetchNextPage}
                    hasNextPage={!!q.hasNextPage}
                    // Only a stream of T's *direct* photos has a source tag; under `all` the photos
                    // come from the whole subtree, so "sibling" is undefined (34 §9).
                    sourceTag={mode === 'all' ? null : path || null}
                    autoExpand={autoExpand}
                    ctx={inner}
                />
            ))}
            <Sentinel
                onReach={q.fetchNextPage}
                active={!!q.hasNextPage && !q.isFetchingNextPage}
                busy={q.isFetchingNextPage}
            />
            {q.isError && (
                <Notice icon={AlertCircle} big={!gated}>
                    {apiErrorMessage(q.error)}
                </Notice>
            )}
            {enabled && !q.isPending && !q.isError && items.length === 0 && children.length === 0 && (
                <Notice icon={ImageOff} big={!gated}>
                    {hideCounts ? 'No photos match the current filters.' : 'No photos match this view.'}
                </Notice>
            )}
        </>
    )

    if (enabled && q.isPending) {
        return (
            <div ref={gateRef} className="flex flex-wrap content-start gap-1.5">
                {Array.from({length: gated ? 6 : 18}).map((_, i) => (
                    <div
                        key={i}
                        className="animate-pulse rounded-[3px] bg-muted"
                        style={{height: rowHeight, flexBasis: `${rowHeight * 1.4}px`, flexGrow: rowHeight * 1.4}}
                    />
                ))}
            </div>
        )
    }

    // Spacing between sections is a gap, never a margin: a margin on a sticky header offsets where it
    // pins and leaves a strip of scrolling content above it.
    return (
        <div ref={gateRef} className="flex flex-col gap-3">
            {enabled ? body : <div className="h-24"/>}
        </div>
    )
}

/** Empty / error copy: a centred block for the whole grid, one quiet line inside a nested section. */
function Notice({icon: Icon, big, children}: {
    icon: typeof ImageOff
    big: boolean
    children: ReactNode
}) {
    if (!big) {
        return (
            <p className="flex items-center gap-1.5 px-1 py-3 text-xs text-muted-foreground">
                <Icon className="h-3.5 w-3.5 shrink-0"/>
                {children}
            </p>
        )
    }
    return (
        <div className="flex flex-col items-center justify-center gap-2 p-10 text-center text-sm text-muted-foreground">
            <Icon className="h-8 w-8"/>
            <p>{children}</p>
        </div>
    )
}

/** Pages the owning stream in as it nears the end — one per stream, so a nested section advances
 *  its own query rather than a global one (§8). */
function Sentinel({onReach, active, busy}: { onReach: () => void; active: boolean; busy: boolean }) {
    const ref = useRef<HTMLDivElement>(null)
    const visible = useInView(ref, {rootMargin: '400px'})
    useEffect(() => {
        if (visible && active) onReach()
    }, [visible, active, onReach])
    // Collapses once the stream is exhausted, so a nested block doesn't end in dead space.
    return (
        <div ref={ref} className={cn('flex items-center justify-center', active || busy ? 'h-10' : 'h-0')}>
            {busy && <Loader2 className="h-4 w-4 animate-spin text-muted-foreground"/>}
        </div>
    )
}

/**
 * A run of collapsed blocks, plus a full-width panel for each expanded one. An expanded block
 * **leaves** the tile row rather than staying in it with its content spliced underneath: the same
 * card, widened, reads as one object opening rather than two things that have to be related.
 */
function Blocks({
                    blocks,
                    order,
                    autoExpand,
                    stickyTop,
                    ctx,
                }: {
    blocks: TagNode[]
    order: number[]
    autoExpand: boolean
    stickyTop: number
    ctx: StreamContext
}) {
    const {update} = useGalleryParams()
    const overrides = useTimelineExpansion((s) => s.overrides)
    const setExpanded = useTimelineExpansion((s) => s.setExpanded)
    const rowHeight = useUIStore((s) => s.rowHeight)
    const isExpanded = (p: string) => overrides[p] ?? autoExpand
    const open = (path: string) => update({tag: path, hierarchy: null, hpath: ''})

    const collapsed = blocks.filter((c) => !isExpanded(c.path))

    return (
        <>
            {collapsed.length > 0 && (
                <ul className="m-0 grid list-none gap-1.5 p-0 select-none"
                    style={{gridTemplateColumns: `repeat(auto-fill, minmax(${rowHeight * 1.4}px, 1fr))`}}>
                    {collapsed.map((child) => (
                        <SubtagBlock
                            key={child.path}
                            node={child}
                            trash={ctx.trash}
                            rowHeight={rowHeight}
                            hideCount={ctx.hideCounts}
                            onToggle={() => setExpanded(child.path, true)}
                            onOpen={() => open(child.path)}
                            onDrop={ctx.onDropOnTag}
                        />
                    ))}
                </ul>
            )}
            {/* Keyed on the original index so an expanded panel keeps its place in the flat order (§8). */}
            {blocks.map((child, j) =>
                isExpanded(child.path) ? (
                    <ExpandedChild
                        key={child.path}
                        child={child}
                        order={[...order, j]}
                        stickyTop={stickyTop}
                        ctx={ctx}
                        onCollapse={() => setExpanded(child.path, false)}
                        onOpen={() => open(child.path)}
                    />
                ) : null,
            )}
        </>
    )
}

/** The expanded block: the same card widened to full width, with its stream inside it (§5). */
function ExpandedChild({child, order, stickyTop, ctx, onCollapse, onOpen}: {
    child: TagNode
    order: number[]
    stickyTop: number
    ctx: StreamContext
    onCollapse: () => void
    onOpen: () => void
}) {
    const accent = child.meta?.color ?? undefined
    const range = formatRange(
        effectiveDate(child, 'date_from', ctx.trash),
        effectiveDate(child, 'date_to', ctx.trash),
    )

    // Opaque base, tinted by the tag's own colour: a sticky header must not let photos show through,
    // and the tint is what tells you which tag the rows below belong to.
    const tint = accent ? `${accent}2e` : 'var(--color-muted)'

    return (
        // No `overflow-hidden`: it would make this card the scrollport of its own sticky headers,
        // which then never pin. The header rounds its own top corners instead.
        <section className="group/block relative w-full rounded-[3px] border"
                 style={accent ? {borderColor: `${accent}80`} : undefined}>
            <div
                className="sticky flex h-7 items-center gap-1.5 rounded-t-[2px] border-b bg-background pr-1"
                style={{
                    top: stickyTop,
                    zIndex: headerZ(stickyTop),
                    backgroundImage: `linear-gradient(${tint}, ${tint})`,
                    borderColor: accent ? `${accent}55` : undefined,
                }}
            >
                <button
                    onClick={onCollapse}
                    aria-expanded
                    className="flex min-w-0 flex-1 items-center gap-1.5 self-stretch rounded-tl-[2px] px-2 text-left hover:bg-foreground/5"
                >
                    {accent ? (
                        <span className="h-2 w-2 shrink-0 rounded-full" style={{backgroundColor: accent}} aria-hidden/>
                    ) : (
                        <Hash className="h-3 w-3 shrink-0 opacity-70"/>
                    )}
                    <span className="truncate text-xs font-medium">{child.name}</span>
                    <span className="min-w-0 truncate text-[10px] text-muted-foreground">
                        {range}
                        {range && !ctx.hideCounts && ' · '}
                        {!ctx.hideCounts && countsFor(child, ctx.trash).count}
                    </span>
                    <ChevronRight className="ml-auto h-3.5 w-3.5 shrink-0 rotate-90 text-muted-foreground"/>
                </button>
                <OpenTagButton onOpen={onOpen} name={child.name} inline/>
            </div>
            {/* `p-1.5` is `PAD_CARD`: the nested stream's headers bleed back out to the card edges. */}
            <div className="p-1.5">
                <TagStream path={child.path} node={child} order={order} gated
                           stickyTop={stickyTop + HEADER_H} pad={PAD_CARD} {...ctx} />
            </div>
        </section>
    )
}

/** One grouping section: its blocks first, then that section's direct photos (§5.3). */
function GroupSection({
                          section,
                          showHeader,
                          settled,
                          order,
                          stickyTop,
                          pad,
                          blocks,
                          sectionKey,
                          fetchNextPage,
                          hasNextPage,
                          sourceTag,
                          autoExpand,
                          ctx,
                      }: {
    section: Section<PictureListItem>
    showHeader: boolean
    /** A section is a slice of one paginated query, so a count is wrong until the last page lands. */
    settled: boolean
    order: number[]
    stickyTop: number
    pad: number
    blocks: TagNode[]
    sectionKey: string
    fetchNextPage: () => void
    hasNextPage: boolean
    sourceTag: string | null
    autoExpand: boolean
    ctx: StreamContext
}) {
    useRegisterSection(sectionKey, {
        order: [...order, 1],
        items: section.items,
        fetchNextPage,
        hasNextPage,
    })

    return (
        <section className="flex flex-col gap-2">
            {showHeader && (
                // Opaque, full-bleed, and no margin — each of the three is a way for the page to show
                // through around a pinned header.
                <h4 className="sticky flex h-7 items-center gap-2 border-b border-border/60 bg-background text-xs font-medium"
                    style={{top: stickyTop, zIndex: headerZ(stickyTop), ...bleed(pad)}}>
                    <span>{section.label}</span>
                    {settled && section.items.length > 0 && (
                        <span className="text-[10px] font-normal text-muted-foreground">{section.items.length}</span>
                    )}
                </h4>
            )}
            {blocks.length > 0 && (
                <Blocks blocks={blocks} order={[...order, 0]} autoExpand={autoExpand}
                        stickyTop={stickyTop + (showHeader ? HEADER_H : 0)} ctx={ctx}/>
            )}
            <ul className="m-0 flex list-none flex-wrap content-start gap-1.5 p-0 select-none">
                <PhotoCards items={section.items} sourceTag={sourceTag}/>
                {/* Absorbs trailing space so the last row keeps natural sizing. */}
                <li aria-hidden className="h-0" style={{flexGrow: 1e7, flexBasis: 0}}/>
            </ul>
        </section>
    )
}
