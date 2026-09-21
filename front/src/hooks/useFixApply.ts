import {useCallback} from 'react'
import {useQueryClient} from '@tanstack/react-query'
import {editPicture, editReceivedExif} from '@/api/pictures'
import {invalidatePicturesAndTags} from '@/lib/invalidation'
import type {GpsResult} from '@/lib/gpsInterpolation'
import type {ExifEditMode, ExifField, ExifOverrides} from '@/lib/types'

/**
 * A GPS or capture-date value to write onto a target (feature 30 §11). A `null` accuracy removes the
 * target's stale one: it described the old location (feature 36 §4).
 */
export interface FixValue {
    gps_lat?: number
    gps_lng?: number
    gps_alt?: number | null
    gps_accuracy_m?: number | null
    captured_at?: string
}

/** Received-picture apply mode: a private local override, or a propose-to-owner edit (§9). */
export type FixReceivedMode = ExifEditMode

/** The fix value for a derived GPS point, carrying its suggested accuracy (feature 36 §4). */
export function gpsFixValue(g: GpsResult): FixValue {
    return {gps_lat: g.lat, gps_lng: g.lng, gps_alt: g.alt, gps_accuracy_m: g.accuracyM}
}

function toDelta(value: FixValue): { set: Partial<ExifOverrides>; unset: ExifField[] } {
    const set: Partial<ExifOverrides> = {}
    const unset: ExifField[] = []
    if (value.captured_at != null) set.captured_at = value.captured_at
    if (value.gps_lat != null && value.gps_lng != null) {
        set.gps_lat = value.gps_lat
        set.gps_lng = value.gps_lng
        if (value.gps_alt != null) set.gps_alt = value.gps_alt
        // Accuracy is outside the backend's coordinate coupling, so it alone can be dropped here.
        if (value.gps_accuracy_m != null) set.gps_accuracy_m = value.gps_accuracy_m
        else unset.push('gps_accuracy_m')
    }
    return {set, unset}
}

/**
 * Write a fix value onto one picture, routing per type (feature 30 §9/§11): owned →
 * write-through (`POST /pictures/{id}/edit`); received → `POST /pictures/{id}/exif` with the batch's
 * `local | propose` mode. Bulk is this call looped by the caller (per-row progress), so the backend
 * handles the multi-share / multi-owner fan-out itself. Callers invalidate once via `invalidate()`.
 */
export function useFixApply() {
    const queryClient = useQueryClient()

    const applyOne = useCallback(
        async (id: string, owned: boolean, value: FixValue, receivedMode: FixReceivedMode) => {
            const {set, unset} = toDelta(value)
            if (Object.keys(set).length === 0) return
            if (owned) {
                await editPicture(id, {set, clear: unset})
            } else {
                await editReceivedExif(id, {mode: receivedMode, set, empty: unset})
            }
        },
        [],
    )

    const invalidate = useCallback(() => invalidatePicturesAndTags(queryClient), [queryClient])

    return {applyOne, invalidate}
}
