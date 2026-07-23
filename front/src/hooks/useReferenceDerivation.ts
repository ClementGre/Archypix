import {useMemo} from 'react'
import {useQueries} from '@tanstack/react-query'
import {getPicture} from '@/api/pictures'
import {queryKeys} from '@/lib/constants'
import {useFixReference} from '@/stores/fixReference'
import {deriveGps, type GpsAnchor, type GpsResult, naiveToMs} from '@/lib/gpsInterpolation'
import {msToNaive} from '@/lib/fixDate'
import type {FixMode, PictureDetail} from '@/lib/types'

export interface ReferenceDerivation {
    /** Loaded details of the picked references. */
    refDetails: PictureDetail[]
    /** The references that carry GPS, as anchors (GPS mode). */
    refAnchors: GpsAnchor[]
    /** The references' capture instants in epoch ms (date mode). */
    refTimes: number[]
    /** Derived GPS value (copy / interpolate / centroid) for the target time (GPS mode). */
    gpsValue: GpsResult | null
    /** Derived date as epoch ms (mean of the references' instants) and its naive string (date mode). */
    dateMs: number | null
    dateValue: string | null
    /** Still fetching some reference details. */
    loading: boolean
    count: number
}

/**
 * Derive a value from the currently-picked reference photos (feature 30 §7): fetch their details and
 * compute the GPS point (copy for one, time-weighted interpolation for a bracketing pair, else
 * centroid) or the mean date. Shared by the single fix panels and the batch reference panel.
 */
export function useReferenceDerivation(field: FixMode, singleTargetTime: string | null): ReferenceDerivation {
    const refIds = useFixReference((s) => s.refIds)

    const refQueries = useQueries({
        queries: refIds.map((id) => ({queryKey: queryKeys.picture(id), queryFn: () => getPicture(id)})),
    })
    const refDetails = refQueries.map((q) => q.data).filter((d): d is PictureDetail => !!d)
    const loading = refQueries.some((q) => q.isPending) && refIds.length > 0

    const refAnchors: GpsAnchor[] = useMemo(
        () =>
            refDetails
                .filter((d) => d.gps_lat != null && d.gps_lng != null)
                .map((d) => ({lat: d.gps_lat!, lng: d.gps_lng!, alt: d.gps_alt, time: d.captured_at})),
        [refDetails],
    )
    const gpsValue = field === 'gps' && refAnchors.length ? deriveGps(singleTargetTime, refAnchors) : null

    const refTimes = useMemo(
        () => refDetails.map((d) => naiveToMs(d.captured_at)).filter((t): t is number => t != null),
        [refDetails],
    )
    const dateMs = field === 'date' && refTimes.length ? Math.round(refTimes.reduce((a, b) => a + b, 0) / refTimes.length) : null

    return {
        refDetails,
        refAnchors,
        refTimes,
        gpsValue,
        dateMs,
        dateValue: dateMs != null ? msToNaive(dateMs) : null,
        loading,
        count: refIds.length,
    }
}
