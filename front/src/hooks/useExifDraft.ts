import {useEffect, useMemo, useRef, useState} from 'react'
import {toast} from 'sonner'
import {useEditExif} from '@/hooks/usePictureEdit'
import {apiErrorMessage} from '@/api/client'
import type {ExifField, ExifOverrides, PictureDetail} from '@/lib/types'

export interface ExifDraft {
    captured_at: string // "YYYY-MM-DDTHH:MM:SS" or ''
    gps_lat: string
    gps_lng: string
    gps_alt: string
    orientation: string
    camera_brand: string
    camera_model: string
    focal_length_mm: string
    f_number: string
    iso_speed: string
    exposure_time_num: string
    exposure_time_den: string
}

function buildInitial(picture: PictureDetail): ExifDraft {
    const e = picture.exif_data ?? {}
    const str = (key: string) => (e[key] != null ? String(e[key]) : '')
    return {
        captured_at: picture.captured_at ?? '',
        gps_lat: picture.gps_lat != null ? String(picture.gps_lat) : '',
        gps_lng: picture.gps_lng != null ? String(picture.gps_lng) : '',
        gps_alt: picture.gps_alt != null ? String(picture.gps_alt) : '',
        orientation: picture.orientation != null ? String(picture.orientation) : '',
        camera_brand: str('camera_brand'),
        camera_model: str('camera_model'),
        focal_length_mm: str('focal_length_mm'),
        f_number: str('f_number'),
        iso_speed: str('iso_speed'),
        exposure_time_num: str('exposure_time_num'),
        exposure_time_den: str('exposure_time_den'),
    }
}

function buildPayload(
    draft: ExifDraft,
    initial: ExifDraft,
): { set?: Partial<ExifOverrides>; clear?: ExifField[] } | null {
    const set: Partial<ExifOverrides> = {}
    const clear: ExifField[] = []

    function diffText(key: ExifField, cur: string, ini: string) {
        if (cur === ini) return
        if (cur === '') {
            if (ini !== '') clear.push(key)
        } else {
            (set as Record<string, unknown>)[key] = cur
        }
    }

    function diffNum(key: ExifField, cur: string, ini: string, toNum: (s: string) => number = Number) {
        if (cur === ini) return
        if (cur === '') {
            if (ini !== '') clear.push(key)
        } else {
            (set as Record<string, unknown>)[key] = toNum(cur)
        }
    }

    diffText('captured_at', draft.captured_at, initial.captured_at) // already NaiveDateTime string
    diffNum('gps_lat', draft.gps_lat, initial.gps_lat)
    diffNum('gps_lng', draft.gps_lng, initial.gps_lng)
    diffNum('gps_alt', draft.gps_alt, initial.gps_alt, (s) => Math.round(Number(s)))
    // orientation is intentionally excluded — rotate buttons auto-commit it separately.
    diffText('camera_brand', draft.camera_brand, initial.camera_brand)
    diffText('camera_model', draft.camera_model, initial.camera_model)
    diffNum('focal_length_mm', draft.focal_length_mm, initial.focal_length_mm)
    diffNum('f_number', draft.f_number, initial.f_number)
    diffNum('iso_speed', draft.iso_speed, initial.iso_speed, (s) => Math.round(Number(s)))
    diffNum('exposure_time_num', draft.exposure_time_num, initial.exposure_time_num, (s) => Math.round(Number(s)))
    diffNum('exposure_time_den', draft.exposure_time_den, initial.exposure_time_den, (s) => Math.round(Number(s)))

    if (Object.keys(set).length === 0 && clear.length === 0) return null
    return {
        ...(Object.keys(set).length > 0 ? {set} : {}),
        ...(clear.length > 0 ? {clear} : {}),
    }
}

// EXIF orientation rotation transition tables (1–8 per the EXIF spec).
const ROT_CW: Record<number, number> = {1: 6, 6: 3, 3: 8, 8: 1, 2: 7, 7: 4, 4: 5, 5: 2}
const ROT_CCW: Record<number, number> = {1: 8, 8: 3, 3: 6, 6: 1, 2: 5, 5: 4, 4: 7, 7: 2}

// Debounce window before a rotate click is committed to the backend, so a burst
// of clicks results in a single edit.
const ROTATE_COMMIT_DEBOUNCE_MS = 700

export function useExifDraft(picture: PictureDetail) {
    const initialDraft = useMemo(
        () => buildInitial(picture),
        // re-seed when the persisted picture data changes (id + updated_at signature)
        // eslint-disable-next-line react-hooks/exhaustive-deps
        [picture.id, picture.updated_at],
    )

    const [draft, setDraft] = useState<ExifDraft>(initialDraft)

    // Adopt server state whenever the picture signature changes (e.g. after a save).
    const sig = `${picture.id}:${picture.updated_at}`
    const lastSig = useRef(sig)
    if (sig !== lastSig.current) {
        lastSig.current = sig
        setDraft(initialDraft)
    }

    const {mutation, syncing} = useEditExif(picture.id)
    const isSaving = mutation.isPending || syncing

    // orientation is excluded from the manual dirty/save flow — rotate buttons commit it
    // automatically (see the debounce effect below).
    const dirtyKeys = useMemo(
        () =>
            (Object.keys(draft) as Array<keyof ExifDraft>).filter(
                (k) => k !== 'orientation' && draft[k] !== initialDraft[k],
            ),
        [draft, initialDraft],
    )
    const isDirty = dirtyKeys.length > 0

    // Auto-commit orientation after a debounce whenever a rotate click changes it. The ref gates
    // out non-rotate orientation changes (initial seed / re-seed after a save).
    const autoCommitOrientation = useRef(false)
    useEffect(() => {
        if (!autoCommitOrientation.current) return
        autoCommitOrientation.current = false
        const value = draft.orientation ? Math.round(Number(draft.orientation)) : null
        if (value == null) return
        const timer = setTimeout(() => {
            mutation.mutate(
                {set: {orientation: value}},
                {onError: (e) => toast.error('Could not rotate picture', {description: apiErrorMessage(e)})},
            )
        }, ROTATE_COMMIT_DEBOUNCE_MS)
        return () => clearTimeout(timer)
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [draft.orientation])

    function set<K extends keyof ExifDraft>(key: K, value: ExifDraft[K]) {
        setDraft((prev) => ({...prev, [key]: value}))
    }

    function setGps(lat: string, lng: string, alt: string) {
        setDraft((prev) => ({...prev, gps_lat: lat, gps_lng: lng, gps_alt: alt}))
    }

    function reset(key: keyof ExifDraft) {
        setDraft((prev) => ({...prev, [key]: initialDraft[key]}))
    }

    function resetGps() {
        setDraft((prev) => ({
            ...prev,
            gps_lat: initialDraft.gps_lat,
            gps_lng: initialDraft.gps_lng,
            gps_alt: initialDraft.gps_alt,
        }))
    }

    function rotate(direction: 'cw' | 'ccw') {
        autoCommitOrientation.current = true
        setDraft((prev) => {
            const current = prev.orientation ? Number(prev.orientation) : 1
            const valid = current >= 1 && current <= 8 ? current : 1
            const next = direction === 'cw' ? ROT_CW[valid] : ROT_CCW[valid]
            return {...prev, orientation: String(next)}
        })
    }

    function save() {
        const payload = buildPayload(draft, initialDraft)
        if (!payload) return
        mutation.mutate(payload, {
            onError: (e) => toast.error('Could not save EXIF', {description: apiErrorMessage(e)}),
        })
    }

    return {
        draft,
        initialDraft,
        dirtyKeys,
        isDirty,
        isSaving,
        set,
        setGps,
        reset,
        resetGps,
        rotate,
        save,
    }
}
