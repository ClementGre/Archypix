import type {PictureFilters, ScopeParams} from './types'

/** The wire scope arm for a gallery filter — one mapping for the list, browse and selections. */
export function scopeParams(filters: PictureFilters): ScopeParams {
    return {
        owned_only: filters.scope === 'owned' || undefined,
        shared_with_me: filters.scope === 'shared' || undefined,
        trash: filters.trash && filters.trash !== 'exclude' ? filters.trash : undefined,
        captured_after: filters.capturedAfter ?? undefined,
        captured_before: filters.capturedBefore ?? undefined,
        ...(filters.missingAny
            ? {missing_any: true}
            : {
                gps: filters.gps && filters.gps !== 'any' ? filters.gps : undefined,
                capture_date: filters.captureDate && filters.captureDate !== 'any' ? filters.captureDate : undefined,
            }),
        sensitive: filters.sensitive && filters.sensitive !== 'any' ? filters.sensitive : undefined,
    }
}
