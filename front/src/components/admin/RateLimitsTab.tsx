import {AlertTriangle, ShieldCheck} from 'lucide-react'
import {Skeleton} from '@/components/ui/skeleton'
import {SettingsPanel} from '@/components/admin/SettingsPanel'
import {useAdminSettings, useAdminSettingMutations, useRateLimits} from '@/hooks/useAdmin'
import type {RateLimitEventBucket} from '@/lib/types'

/** The runtime-config group the frequency limits live under (`group::RATE_LIMITS`, backend). */
const RATE_LIMIT_GROUP = 'Rate Limits & Caps'
/** How many recent minutes the rejection timeline spans. */
const WINDOW_MINUTES = 60

const CATEGORY_LABELS: Record<string, string> = {
    login: 'Login',
    register: 'Registration',
    public_upload: 'Public upload',
    federation: 'Federation',
    presign: 'Presign',
}

/** A dense last-`WINDOW_MINUTES` bar timeline from the sparse per-minute buckets. */
function Timeline({buckets}: { buckets: RateLimitEventBucket[] }) {
    const nowMinute = Math.floor(Date.now() / 60_000)
    const byMinute = new Map(buckets.map((b) => [b.minute_epoch, b.count]))
    const bars = Array.from({length: WINDOW_MINUTES}, (_, i) => {
        const minute = nowMinute - (WINDOW_MINUTES - 1 - i)
        return {minute, count: byMinute.get(minute) ?? 0}
    })
    const max = Math.max(1, ...bars.map((b) => b.count))
    return (
        <div className="flex h-10 items-end gap-px" aria-hidden>
            {bars.map((b) => (
                <div
                    key={b.minute}
                    className="flex-1 rounded-sm bg-primary/70"
                    style={{height: b.count ? `${(b.count / max) * 100}%` : 0, minHeight: b.count ? 2 : 0}}
                    title={b.count ? `${b.count} rejected · ${new Date(b.minute * 60_000).toLocaleTimeString()}` : undefined}
                />
            ))}
        </div>
    )
}

/** Recent-rejections timeline per category + the attack flag (feature 28 §9.3). */
function RejectionTimelines() {
    const {data, isLoading} = useRateLimits()

    if (isLoading || !data) {
        return <div className="space-y-3">{Array.from({length: 3}).map((_, i) => <Skeleton key={i} className="h-20 w-full"/>)}</div>
    }

    const byCategory = new Map<string, RateLimitEventBucket[]>()
    for (const b of data.buckets) {
        const list = byCategory.get(b.category) ?? []
        list.push(b)
        byCategory.set(b.category, list)
    }
    const categories = [...byCategory.keys()].sort()

    return (
        <div className="space-y-3">
            {data.attack_suspected ? (
                <div className="flex items-center gap-2 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                    <AlertTriangle className="h-4 w-4 shrink-0"/>
                    <span>Possible attack in progress. A category exceeded its rejection threshold in the last few minutes.</span>
                </div>
            ) : (
                <div className="flex items-center gap-2 rounded-md border border-border bg-muted/30 px-3 py-2 text-sm text-muted-foreground">
                    <ShieldCheck className="h-4 w-4 shrink-0 text-emerald-500"/>
                    <span>No unusual rejection activity.</span>
                </div>
            )}

            {categories.length === 0 ? (
                <p className="text-sm text-muted-foreground">No rate-limit rejections recorded in the retention window.</p>
            ) : (
                <div className="grid gap-3 sm:grid-cols-2">
                    {categories.map((cat) => {
                        const buckets = byCategory.get(cat)!
                        const total = buckets.reduce((sum, b) => sum + b.count, 0)
                        return (
                            <div key={cat} className="rounded-lg border border-border p-3">
                                <div className="mb-2 flex items-baseline justify-between">
                                    <span className="text-sm font-medium">{CATEGORY_LABELS[cat] ?? cat}</span>
                                    <span className="text-xs text-muted-foreground">{total} rejected</span>
                                </div>
                                <Timeline buckets={buckets}/>
                                <p className="mt-1 text-[10px] text-muted-foreground">Last {WINDOW_MINUTES} min</p>
                            </div>
                        )
                    })}
                </div>
            )}
        </div>
    )
}

/** The runtime frequency limits (`RATE_LIMITS` group) via the shared metadata-driven panel. */
function RateLimitSettings() {
    const {data, isLoading} = useAdminSettings()
    const {patch, reset} = useAdminSettingMutations()

    if (isLoading || !data) {
        return <div className="space-y-4">{Array.from({length: 3}).map((_, i) => <Skeleton key={i} className="h-24 w-full"/>)}</div>
    }

    const fields = data.filter((f) => f.group === RATE_LIMIT_GROUP)
    return (
        <SettingsPanel
            fields={fields}
            onPatch={(key, value) => patch.mutateAsync({key, value}).then(() => undefined)}
            onReset={(key) => reset.mutateAsync(key).then(() => undefined)}
        />
    )
}

/**
 * Admin "Rate limiting" tab (feature 28 §9.3): the recent-rejections timeline + attack flag, then the
 * editable frequency-limit settings. Structural batch ceilings are hardcoded backend consts (not shown).
 */
export function RateLimitsTab() {
    return (
        <div className="space-y-6">
            <div>
                <h3 className="mb-2 text-sm font-semibold">Recent rejections</h3>
                <RejectionTimelines/>
            </div>
            <div>
                <h3 className="mb-2 text-sm font-semibold">Limits</h3>
                <p className="mb-3 text-sm text-muted-foreground">
                    Frequency limits are large by default and never trip normal behaviour. Fields set by an
                    environment variable are locked and read-only.
                </p>
                <RateLimitSettings/>
            </div>
        </div>
    )
}
