import {useCallback} from 'react'
import {useQueryClient} from '@tanstack/react-query'
import {editPicture, editReceivedExif} from '@/api/pictures'
import {invalidatePicturesAndTags} from '@/lib/invalidation'
import type {ExifEditMode, ExifOverrides} from '@/lib/types'

/** A GPS or capture-date value to write onto a target (feature 30 §11). */
export interface FixValue {
    gps_lat?: number
    gps_lng?: number
    gps_alt?: number | null
    captured_at?: string
}

/** Received-picture apply mode: a private local override, or a propose-to-owner edit (§9). */
export type FixReceivedMode = ExifEditMode

function toSet(value: FixValue): Partial<ExifOverrides> {
    const set: Partial<ExifOverrides> = {}
    if (value.gps_lat != null) set.gps_lat = value.gps_lat
    if (value.gps_lng != null) set.gps_lng = value.gps_lng
    if (value.gps_alt != null) set.gps_alt = value.gps_alt
    if (value.captured_at != null) set.captured_at = value.captured_at
    return set
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
            const set = toSet(value)
            if (Object.keys(set).length === 0) return
            if (owned) {
                await editPicture(id, {set})
            } else {
                await editReceivedExif(id, {mode: receivedMode, set})
            }
        },
        [],
    )

    const invalidate = useCallback(() => invalidatePicturesAndTags(queryClient), [queryClient])

    return {applyOne, invalidate}
}
