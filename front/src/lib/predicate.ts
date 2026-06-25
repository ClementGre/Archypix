// Helpers for the structured rule predicate tree (feature 13) — field metadata, operator
// definitions, a default-builder, serialization from the editor's internal node tree, and a
// human-readable describer for displaying stored predicates.
//
// See doc/features/13_better_rules.md for the model the backend validates against.

import type {FieldPredicate, GpsBbox, GpsRadius, RulePredicate} from './types'

export type FieldType = 'int' | 'float' | 'str' | 'date' | 'bool'

/** Category a field belongs to, for the grouped field picker. */
export type FieldGroup = 'Dates' | 'Camera' | 'Location' | 'File' | 'Ownership'

/** Ordered list of groups for the picker. */
export const FIELD_GROUPS: FieldGroup[] = ['Dates', 'Camera', 'Location', 'File', 'Ownership']

export interface FieldDef {
    name: string
    label: string
    type: FieldType
    group: FieldGroup
    /** Suffix shown after a numeric input (e.g. `mm`, `s`). */
    unit?: string
}

/** Every queryable field, mirroring the backend `Field` enum. */
export const RULE_FIELDS: FieldDef[] = [
    {name: 'captured_at', label: 'Capture date', type: 'date', group: 'Dates'},
    {name: 'ingested_at', label: 'Ingestion date', type: 'date', group: 'Dates'},
    {name: 'updated_at', label: 'Last edited', type: 'date', group: 'Dates'},
    {name: 'camera_brand', label: 'Camera brand', type: 'str', group: 'Camera'},
    {name: 'camera_model', label: 'Camera model', type: 'str', group: 'Camera'},
    {name: 'iso_speed', label: 'ISO', type: 'int', group: 'Camera'},
    {name: 'f_number', label: 'f-number', type: 'float', group: 'Camera'},
    {name: 'focal_length_mm', label: 'Focal length', type: 'float', unit: 'mm', group: 'Camera'},
    {name: 'exposure_time', label: 'Exposure time', type: 'float', unit: 's', group: 'Camera'},
    {name: 'orientation', label: 'Orientation', type: 'int', group: 'Camera'},
    {name: 'gps_lat', label: 'GPS latitude', type: 'float', group: 'Location'},
    {name: 'gps_lng', label: 'GPS longitude', type: 'float', group: 'Location'},
    {name: 'gps_alt', label: 'GPS altitude', type: 'int', unit: 'm', group: 'Location'},
    {name: 'filename', label: 'Filename', type: 'str', group: 'File'},
    {name: 'mime_type', label: 'MIME type', type: 'str', group: 'File'},
    {name: 'file_size', label: 'File size', type: 'int', unit: 'bytes', group: 'File'},
    {name: 'width', label: 'Width', type: 'int', unit: 'px', group: 'File'},
    {name: 'height', label: 'Height', type: 'int', unit: 'px', group: 'File'},
    {name: 'is_owned', label: 'Owned by me', type: 'bool', group: 'Ownership'},
]

export function fieldsByGroup(group: FieldGroup): FieldDef[] {
    return RULE_FIELDS.filter((f) => f.group === group)
}

export function fieldDef(name: string): FieldDef | undefined {
    return RULE_FIELDS.find((f) => f.name === name)
}

/** A condition operator the field-leaf editor offers per base type. */
export interface OperatorDef {
    op: string
    label: string
}

// `is_present` / `is_absent` are two separate operators (not one operator plus a set/not-set
// dropdown), so the user picks "is set" or "is not set" directly. Both serialize to the
// backend's `is_present` boolean leaf.
const NUM_OPS: OperatorDef[] = [
    {op: 'eq', label: 'equals'},
    {op: 'min', label: 'at least'},
    {op: 'max', label: 'at most'},
    {op: 'between', label: 'between'},
    {op: 'is_present', label: 'is set'},
    {op: 'is_absent', label: 'is not set'},
]
// `eq_ic` is gone — case sensitivity is the per-leaf `ignore_case` checkbox (feature 15).
const STR_OPS: OperatorDef[] = [
    {op: 'contains', label: 'contains'},
    {op: 'starts_with', label: 'starts with'},
    {op: 'ends_with', label: 'ends with'},
    {op: 'eq', label: 'equals'},
    {op: 'regex', label: 'matches regex'},
    {op: 'is_present', label: 'is set'},
    {op: 'is_absent', label: 'is not set'},
]

/** String operators that honour the `ignore_case` flag (all comparisons; not presence). */
export const IGNORE_CASE_OPS = new Set(['contains', 'starts_with', 'ends_with', 'eq', 'regex'])
const DATE_OPS: OperatorDef[] = [
    {op: 'year', label: 'in year'},
    {op: 'month', label: 'in month'},
    {op: 'season', label: 'in season'},
    {op: 'date_range', label: 'in date range'},
    {op: 'time_range', label: 'in time range'},
    {op: 'is_present', label: 'is set'},
    {op: 'is_absent', label: 'is not set'},
]
const BOOL_OPS: OperatorDef[] = [{op: 'eq', label: 'is'}]

export function operatorsFor(type: FieldType): OperatorDef[] {
    switch (type) {
        case 'int':
        case 'float':
            return NUM_OPS
        case 'str':
            return STR_OPS
        case 'date':
            return DATE_OPS
        case 'bool':
            return BOOL_OPS
    }
}

export const SEASONS = ['spring', 'summer', 'autumn', 'winter'] as const

// ── Editor internal tree ────────────────────────────────────────────────────────
//
// The builder works on a tree of `BNode`s carrying a stable `id` (for React keys + drag-and-drop)
// and editor-friendly value state. It is serialized to a `RulePredicate` on save.

let nodeSeq = 0
const newId = () => `n${nodeSeq++}`

/** Condition state for a field leaf: an operator plus its operand values. */
export interface CondState {
    op: string
    /** Single value operand (eq/min/max/year/month/contains/regex/season/bool…). */
    value?: string
    /** Second operand for `between` (max) / range upper bound. */
    value2?: string
    /** Range lower bound (date_range.from / time_range.from). */
    from?: string
    /** Range upper bound (date_range.to / time_range.to). */
    to?: string
    /** Case-insensitive matching for string comparisons (serialized as the `ignore_case` flag). */
    ignoreCase?: boolean
}

export interface GroupNode {
    id: string
    kind: 'group'
    op: 'and' | 'or'
    children: BNode[]
}

export type BNode =
    | GroupNode
    | { id: string; kind: 'not'; child: BNode }
    | { id: string; kind: 'field'; field: string; cond: CondState }
    | { id: string; kind: 'gps_bbox'; box: GpsBbox }
    | { id: string; kind: 'gps_radius'; radius: GpsRadius }

export function defaultCond(type: FieldType): CondState {
    const op = operatorsFor(type)[0].op
    if (op === 'is_present') return {op, value: 'true'}
    if (type === 'bool') return {op: 'eq', value: 'true'}
    return {op, value: ''}
}

export function newFieldNode(field = 'camera_brand'): BNode {
    const def = fieldDef(field)!
    return {id: newId(), kind: 'field', field, cond: defaultCond(def.type)}
}

export function newGroupNode(op: 'and' | 'or' = 'and'): GroupNode {
    return {id: newId(), kind: 'group', op, children: []}
}

export function newNotNode(): BNode {
    return {id: newId(), kind: 'not', child: newFieldNode()}
}

export function newGpsBboxNode(): BNode {
    return {id: newId(), kind: 'gps_bbox', box: {lat_min: 0, lat_max: 0, lon_min: 0, lon_max: 0}}
}

export function newGpsRadiusNode(): BNode {
    return {id: newId(), kind: 'gps_radius', radius: {lat: 0, lng: 0, km: 10}}
}

/** The default predicate for a brand-new rule: an empty AND group (matches everything). */
export function newRootNode(): BNode {
    return newGroupNode('and')
}

// ── Serialization (BNode → RulePredicate JSON) ──────────────────────────────────

function num(s: string | undefined): number {
    const n = Number(s)
    return Number.isFinite(n) ? n : 0
}

function condToObject(field: string, cond: CondState, type: FieldType): FieldPredicate {
    const base: FieldPredicate = {field}
    switch (cond.op) {
        case 'is_present':
            base.is_present = true
            break
        case 'is_absent':
            base.is_present = false
            break
        case 'eq':
            if (type === 'bool') base.eq = cond.value === 'true'
            else if (type === 'str') base.eq = cond.value ?? ''
            else base.eq = num(cond.value)
            break
        case 'min':
            base.min = num(cond.value)
            break
        case 'max':
            base.max = num(cond.value)
            break
        case 'between':
            base.min = num(cond.value)
            base.max = num(cond.value2)
            break
        case 'contains':
            base.contains = cond.value ?? ''
            break
        case 'starts_with':
            base.starts_with = cond.value ?? ''
            break
        case 'ends_with':
            base.ends_with = cond.value ?? ''
            break
        case 'regex':
            base.regex = cond.value ?? ''
            break
        case 'year':
            base.year = num(cond.value)
            break
        case 'month':
            base.month = num(cond.value)
            break
        case 'season':
            base.season = cond.value ?? 'summer'
            break
        case 'date_range':
            base.date_range = {from: cond.from ?? '', to: cond.to ?? ''}
            break
        case 'time_range':
            base.time_range = {from: cond.from ?? '', to: cond.to ?? ''}
            break
    }
    // Case-insensitivity is a sibling flag on string comparisons.
    if (type === 'str' && cond.ignoreCase && IGNORE_CASE_OPS.has(cond.op)) {
        base.ignore_case = true
    }
    return base
}

export function serialize(node: BNode): RulePredicate {
    switch (node.kind) {
        case 'group':
            return node.op === 'and'
                ? {and: node.children.map(serialize)}
                : {or: node.children.map(serialize)}
        case 'not':
            return {not: serialize(node.child)}
        case 'field': {
            const def = fieldDef(node.field)!
            return condToObject(node.field, node.cond, def.type)
        }
        case 'gps_bbox':
            return {gps_bbox: node.box}
        case 'gps_radius':
            return {gps_radius: node.radius}
    }
}

// ── Deserialization (RulePredicate JSON → editor tree) ──────────────────────────
//
// Used when editing an existing rule: hydrate the stored predicate into a `BNode` tree.

export function deserialize(p: RulePredicate): BNode {
    if (p && typeof p === 'object') {
        if ('and' in p) {
            return {id: newId(), kind: 'group', op: 'and', children: (p.and as RulePredicate[]).map(deserialize)}
        }
        if ('or' in p) {
            return {id: newId(), kind: 'group', op: 'or', children: (p.or as RulePredicate[]).map(deserialize)}
        }
        if ('not' in p) {
            return {id: newId(), kind: 'not', child: deserialize(p.not as RulePredicate)}
        }
        if ('gps_bbox' in p) {
            return {id: newId(), kind: 'gps_bbox', box: p.gps_bbox as GpsBbox}
        }
        if ('gps_radius' in p) {
            return {id: newId(), kind: 'gps_radius', radius: p.gps_radius as GpsRadius}
        }
        if ('field' in p) {
            return fieldNodeFrom(p as FieldPredicate)
        }
    }
    // Unrecognised — fall back to an empty AND (matches everything) rather than crashing.
    return newGroupNode('and')
}

function fieldNodeFrom(p: FieldPredicate): BNode {
    const field = fieldDef(p.field) ? p.field : 'camera_brand'
    const s = (v: unknown) => (v == null ? '' : String(v))
    let cond: CondState
    if ('is_present' in p) cond = {op: p.is_present ? 'is_present' : 'is_absent'}
    else if ('min' in p && 'max' in p) cond = {op: 'between', value: s(p.min), value2: s(p.max)}
    else if ('min' in p) cond = {op: 'min', value: s(p.min)}
    else if ('max' in p) cond = {op: 'max', value: s(p.max)}
    else if ('eq' in p) cond = {op: 'eq', value: typeof p.eq === 'boolean' ? (p.eq ? 'true' : 'false') : s(p.eq)}
    // Legacy `eq_ic` (pre-feature-15) maps to eq + ignore_case for any un-migrated predicate.
    else if ('eq_ic' in p) cond = {op: 'eq', value: s(p.eq_ic), ignoreCase: true}
    else if ('contains' in p) cond = {op: 'contains', value: s(p.contains)}
    else if ('starts_with' in p) cond = {op: 'starts_with', value: s(p.starts_with)}
    else if ('ends_with' in p) cond = {op: 'ends_with', value: s(p.ends_with)}
    else if ('regex' in p) cond = {op: 'regex', value: s(p.regex)}
    else if ('year' in p) cond = {op: 'year', value: s(p.year)}
    else if ('month' in p) cond = {op: 'month', value: s(p.month)}
    else if ('season' in p) cond = {op: 'season', value: s(p.season)}
    else if ('date_range' in p) {
        const r = p.date_range as { from: string; to: string }
        cond = {op: 'date_range', from: r.from, to: r.to}
    } else if ('time_range' in p) {
        const r = p.time_range as { from: string; to: string }
        cond = {op: 'time_range', from: r.from, to: r.to}
    } else {
        cond = defaultCond(fieldDef(field)!.type)
    }
    // Carry the explicit `ignore_case` flag (sibling key) onto string comparisons.
    if ((p as Record<string, unknown>).ignore_case === true && IGNORE_CASE_OPS.has(cond.op)) {
        cond.ignoreCase = true
    }
    return {id: newId(), kind: 'field', field, cond}
}

// ── Tree moves (drag predicates between levels) ─────────────────────────────────
//
// Containers are the root and every group node; a NOT's single child is not a sortable item.

/** Find the group container holding `id` as a direct child, and that child's index. */
export function locate(root: BNode, id: string): { containerId: string; index: number } | null {
    function walk(node: BNode): { containerId: string; index: number } | null {
        if (node.kind === 'group') {
            const idx = node.children.findIndex((c) => c.id === id)
            if (idx >= 0) return {containerId: node.id, index: idx}
            for (const c of node.children) {
                const r = walk(c)
                if (r) return r
            }
        } else if (node.kind === 'not') {
            return walk(node.child)
        }
        return null
    }

    return walk(root)
}

/** Remove the node with `id` from its group parent. Returns the new tree and the removed node. */
export function detach(root: BNode, id: string): { tree: BNode; node: BNode | null } {
    let removed: BNode | null = null

    function walk(node: BNode): BNode {
        if (node.kind === 'group') {
            const children: BNode[] = []
            for (const c of node.children) {
                if (c.id === id) {
                    removed = c
                    continue
                }
                children.push(walk(c))
            }
            return {...node, children}
        }
        if (node.kind === 'not') return {...node, child: walk(node.child)}
        return node
    }

    const tree = walk(root)
    return {tree, node: removed}
}

/** Insert `node` into the group `containerId` at `index`. */
export function insertInto(root: BNode, containerId: string, index: number, node: BNode): BNode {
    function walk(n: BNode): BNode {
        if (n.kind === 'group') {
            if (n.id === containerId) {
                const children = [...n.children]
                children.splice(Math.max(0, Math.min(index, children.length)), 0, node)
                return {...n, children}
            }
            return {...n, children: n.children.map(walk)}
        }
        if (n.kind === 'not') return {...n, child: walk(n.child)}
        return n
    }

    return walk(root)
}

/** Is `descendantId` inside the subtree rooted at `ancestorId` (preventing a self-drop)? */
export function isWithin(root: BNode, ancestorId: string, descendantId: string): boolean {
    let ancestor: BNode | null = null

    function find(n: BNode) {
        if (n.id === ancestorId) ancestor = n
        else if (n.kind === 'group') n.children.forEach(find)
        else if (n.kind === 'not') find(n.child)
    }

    find(root)
    if (!ancestor) return false

    function contains(n: BNode): boolean {
        if (n.id === descendantId) return true
        if (n.kind === 'group') return n.children.some(contains)
        if (n.kind === 'not') return contains(n.child)
        return false
    }

    return ancestor !== null && (ancestor as BNode).id !== descendantId && contains(ancestor)
}

// ── Human-readable describer (RulePredicate JSON → string) ───────────────────────

function fieldLabel(name: string): string {
    return fieldDef(name)?.label ?? name
}

/** Render a stored predicate as a compact, readable expression for the rule list. */
export function describePredicate(p: RulePredicate): string {
    if (p && typeof p === 'object') {
        if ('and' in p) {
            const parts = (p.and as RulePredicate[]).map(describePredicate)
            return parts.length === 0 ? 'always' : parts.map(wrap).join(' AND ')
        }
        if ('or' in p) {
            const parts = (p.or as RulePredicate[]).map(describePredicate)
            return parts.length === 0 ? 'never' : parts.map(wrap).join(' OR ')
        }
        if ('not' in p) {
            return `NOT ${wrap(describePredicate(p.not as RulePredicate))}`
        }
        if ('gps_bbox' in p) {
            const b = p.gps_bbox as GpsBbox
            return `GPS in box [${b.lat_min}, ${b.lat_max}] × [${b.lon_min}, ${b.lon_max}]`
        }
        if ('gps_radius' in p) {
            const r = p.gps_radius as GpsRadius
            return `GPS within ${r.km} km of ${r.lat}, ${r.lng}`
        }
        if ('field' in p) {
            return describeField(p as FieldPredicate)
        }
    }
    return '(invalid)'
}

function wrap(s: string): string {
    return s.includes(' AND ') || s.includes(' OR ') ? `(${s})` : s
}

function describeField(p: FieldPredicate): string {
    const label = fieldLabel(p.field)
    // Appended to string comparisons that fold case (the `ignore_case` sibling flag).
    const ic = (p as Record<string, unknown>).ignore_case === true ? ' (ignore case)' : ''
    if ('is_present' in p) return p.is_present ? `${label} is set` : `${label} is not set`
    if ('eq' in p) return typeof p.eq === 'string' ? `${label} = "${p.eq}"${ic}` : `${label} = ${fmt(p.eq)}`
    if ('eq_ic' in p) return `${label} ≈ ${fmt(p.eq_ic)}`
    if ('contains' in p) return `${label} contains "${p.contains}"${ic}`
    if ('starts_with' in p) return `${label} starts with "${p.starts_with}"${ic}`
    if ('ends_with' in p) return `${label} ends with "${p.ends_with}"${ic}`
    if ('regex' in p) return `${label} matches /${p.regex}/${ic}`
    if ('min' in p && 'max' in p) return `${label} ∈ [${fmt(p.min)}, ${fmt(p.max)}]`
    if ('min' in p) return `${label} ≥ ${fmt(p.min)}`
    if ('max' in p) return `${label} ≤ ${fmt(p.max)}`
    if ('year' in p) return `${label} in ${fmt(p.year)}`
    if ('month' in p) return `${label} in month ${fmt(p.month)}`
    if ('season' in p) return `${label} in ${p.season}`
    if ('date_range' in p) {
        const r = p.date_range as { from: string; to: string }
        return `${label} ${r.from} → ${r.to}`
    }
    if ('time_range' in p) {
        const r = p.time_range as { from: string; to: string }
        return `${label} ${r.from} → ${r.to}`
    }
    return label
}

function fmt(v: unknown): string {
    if (typeof v === 'boolean') return v ? 'yes' : 'no'
    return String(v)
}
