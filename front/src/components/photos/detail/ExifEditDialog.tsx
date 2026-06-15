import {useState} from 'react'
import {Loader2, Pencil} from 'lucide-react'
import {toast} from 'sonner'
import {Dialog, DialogContent, DialogHeader, DialogTitle, DialogTrigger} from '@/components/ui/dialog'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {Label} from '@/components/ui/label'
import {useEditExif} from '@/hooks/usePictureEdit'
import {apiErrorMessage} from '@/api/client'
import type {ExifField, ExifOverrides, PictureDetail} from '@/lib/types'

// ── helpers ──────────────────────────────────────────────────────────────────

/** Convert an ISO string to the value accepted by <input type="datetime-local">. */
function isoToDatetimeLocal(iso: string | null | undefined): string {
    if (!iso) return ''
    // datetime-local expects "YYYY-MM-DDTHH:MM" (no seconds / timezone)
    return iso.slice(0, 16)
}

/** Convert a datetime-local input value back to an ISO UTC string. */
function datetimeLocalToIso(value: string): string {
    return new Date(value).toISOString()
}

/** Pull a string from picture.exif_data by key, or return ''. */
function exifStr(exif_data: Record<string, unknown>, key: string): string {
    const v = exif_data[key]
    return v != null ? String(v) : ''
}

/** Pull a numeric value from picture.exif_data as a string, or return ''. */
function exifNum(exif_data: Record<string, unknown>, key: string): string {
    const v = exif_data[key]
    return v != null ? String(v) : ''
}

// ── types ────────────────────────────────────────────────────────────────────

interface FieldState {
    captured_at: string        // datetime-local string or ''
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

function buildInitialState(picture: PictureDetail): FieldState {
    const e = picture.exif_data
    return {
        captured_at: isoToDatetimeLocal(picture.captured_at),
        gps_lat: picture.gps_lat != null ? String(picture.gps_lat) : '',
        gps_lng: picture.gps_lng != null ? String(picture.gps_lng) : '',
        gps_alt: picture.gps_alt != null ? String(picture.gps_alt) : '',
        orientation: picture.orientation != null ? String(picture.orientation) : '',
        camera_brand: exifStr(e, 'camera_brand'),
        camera_model: exifStr(e, 'camera_model'),
        focal_length_mm: exifNum(e, 'focal_length_mm'),
        f_number: exifNum(e, 'f_number'),
        iso_speed: exifNum(e, 'iso_speed'),
        exposure_time_num: exifNum(e, 'exposure_time_num'),
        exposure_time_den: exifNum(e, 'exposure_time_den'),
    }
}

// ── component ─────────────────────────────────────────────────────────────────

export function ExifEditDialog({picture}: { picture: PictureDetail }) {
    const [open, setOpen] = useState(false)

    const initialState = buildInitialState(picture)
    const [fields, setFields] = useState<FieldState>(initialState)

    const {mutation, syncing} = useEditExif(picture.id)

    const isPending = mutation.isPending || syncing

    function setField(key: keyof FieldState, value: string) {
        setFields((prev) => ({...prev, [key]: value}))
    }

    function handleOpenChange(next: boolean) {
        if (next) {
            // Re-seed from picture each time dialog opens.
            setFields(buildInitialState(picture))
        }
        setOpen(next)
    }

    function handleSubmit(e: React.FormEvent) {
        e.preventDefault()

        const set: Partial<ExifOverrides> = {}
        const clear: ExifField[] = []

        // For each field: compare current string value against the initial value.
        // • Changed to non-empty  → add to `set` (with appropriate type coercion).
        // • Was non-empty, now empty → add to `clear`.
        // • Unchanged (both empty or both same value) → skip.

        function diffText(key: ExifField, current: string, initial: string) {
            if (current === initial) return
            if (current === '') {
                if (initial !== '') clear.push(key)
            } else {
                (set as Record<string, unknown>)[key] = current
            }
        }

        function diffNum(key: ExifField, current: string, initial: string, toNumber: (s: string) => number = Number) {
            if (current === initial) return
            if (current === '') {
                if (initial !== '') clear.push(key)
            } else {
                (set as Record<string, unknown>)[key] = toNumber(current)
            }
        }

        function diffDatetime(key: ExifField, current: string, initial: string) {
            if (current === initial) return
            if (current === '') {
                if (initial !== '') clear.push(key)
            } else {
                (set as Record<string, unknown>)[key] = datetimeLocalToIso(current)
            }
        }

        diffDatetime('captured_at', fields.captured_at, initialState.captured_at)
        diffNum('gps_lat', fields.gps_lat, initialState.gps_lat)
        diffNum('gps_lng', fields.gps_lng, initialState.gps_lng)
        diffNum('gps_alt', fields.gps_alt, initialState.gps_alt, (s) => Math.round(Number(s)))
        diffNum('orientation', fields.orientation, initialState.orientation, (s) => Math.round(Number(s)))
        diffText('camera_brand', fields.camera_brand, initialState.camera_brand)
        diffText('camera_model', fields.camera_model, initialState.camera_model)
        diffNum('focal_length_mm', fields.focal_length_mm, initialState.focal_length_mm)
        diffNum('f_number', fields.f_number, initialState.f_number)
        diffNum('iso_speed', fields.iso_speed, initialState.iso_speed, (s) => Math.round(Number(s)))
        diffNum('exposure_time_num', fields.exposure_time_num, initialState.exposure_time_num, (s) => Math.round(Number(s)))
        diffNum('exposure_time_den', fields.exposure_time_den, initialState.exposure_time_den, (s) => Math.round(Number(s)))

        if (Object.keys(set).length === 0 && clear.length === 0) {
            toast.info('No changes to save')
            setOpen(false)
            return
        }

        mutation.mutate(
            {
                ...(Object.keys(set).length > 0 ? {set} : {}),
                ...(clear.length > 0 ? {clear} : {}),
            },
            {
                onSuccess: () => {
                    if (!syncing) setOpen(false)
                },
                onError: (error) => {
                    toast.error('Could not save EXIF', {description: apiErrorMessage(error)})
                },
            },
        )
    }

    // Close dialog once a pending sync resolves (syncing transitions false→false after open was kept).
    // We rely on the mutation onSuccess to close the dialog; syncing=true keeps it open while polling.

    return (
        <Dialog open={open} onOpenChange={handleOpenChange}>
            <DialogTrigger asChild>
                <Button variant="ghost" size="icon" className="h-7 w-7" aria-label="Edit EXIF">
                    <Pencil className="h-4 w-4"/>
                </Button>
            </DialogTrigger>

            <DialogContent className="max-w-lg max-h-[90vh] overflow-y-auto">
                <DialogHeader>
                    <DialogTitle>Edit EXIF</DialogTitle>
                </DialogHeader>

                <form onSubmit={handleSubmit} className="space-y-4">
                    {/* Capture date/time */}
                    <div className="space-y-1.5">
                        <Label htmlFor="exif-captured-at">Captured at</Label>
                        <Input
                            id="exif-captured-at"
                            type="datetime-local"
                            value={fields.captured_at}
                            onChange={(e) => setField('captured_at', e.target.value)}
                        />
                    </div>

                    {/* GPS */}
                    <div className="grid grid-cols-3 gap-3">
                        <div className="space-y-1.5">
                            <Label htmlFor="exif-gps-lat">Latitude</Label>
                            <Input
                                id="exif-gps-lat"
                                type="number"
                                step="any"
                                placeholder="e.g. 48.8566"
                                value={fields.gps_lat}
                                onChange={(e) => setField('gps_lat', e.target.value)}
                            />
                        </div>
                        <div className="space-y-1.5">
                            <Label htmlFor="exif-gps-lng">Longitude</Label>
                            <Input
                                id="exif-gps-lng"
                                type="number"
                                step="any"
                                placeholder="e.g. 2.3522"
                                value={fields.gps_lng}
                                onChange={(e) => setField('gps_lng', e.target.value)}
                            />
                        </div>
                        <div className="space-y-1.5">
                            <Label htmlFor="exif-gps-alt">Altitude (m)</Label>
                            <Input
                                id="exif-gps-alt"
                                type="number"
                                step="1"
                                placeholder="e.g. 35"
                                value={fields.gps_alt}
                                onChange={(e) => setField('gps_alt', e.target.value)}
                            />
                        </div>
                    </div>

                    {/* Orientation */}
                    <div className="space-y-1.5">
                        <Label htmlFor="exif-orientation">Orientation (1–8)</Label>
                        <Input
                            id="exif-orientation"
                            type="number"
                            min={1}
                            max={8}
                            step={1}
                            value={fields.orientation}
                            onChange={(e) => setField('orientation', e.target.value)}
                        />
                    </div>

                    {/* Camera */}
                    <div className="grid grid-cols-2 gap-3">
                        <div className="space-y-1.5">
                            <Label htmlFor="exif-camera-brand">Camera brand</Label>
                            <Input
                                id="exif-camera-brand"
                                value={fields.camera_brand}
                                onChange={(e) => setField('camera_brand', e.target.value)}
                                placeholder="e.g. Canon"
                            />
                        </div>
                        <div className="space-y-1.5">
                            <Label htmlFor="exif-camera-model">Camera model</Label>
                            <Input
                                id="exif-camera-model"
                                value={fields.camera_model}
                                onChange={(e) => setField('camera_model', e.target.value)}
                                placeholder="e.g. EOS R5"
                            />
                        </div>
                    </div>

                    {/* Lens / exposure */}
                    <div className="grid grid-cols-2 gap-3">
                        <div className="space-y-1.5">
                            <Label htmlFor="exif-focal-length">Focal length (mm)</Label>
                            <Input
                                id="exif-focal-length"
                                type="number"
                                step="any"
                                placeholder="e.g. 50"
                                value={fields.focal_length_mm}
                                onChange={(e) => setField('focal_length_mm', e.target.value)}
                            />
                        </div>
                        <div className="space-y-1.5">
                            <Label htmlFor="exif-f-number">f-number</Label>
                            <Input
                                id="exif-f-number"
                                type="number"
                                step="any"
                                placeholder="e.g. 1.8"
                                value={fields.f_number}
                                onChange={(e) => setField('f_number', e.target.value)}
                            />
                        </div>
                    </div>

                    <div className="grid grid-cols-3 gap-3">
                        <div className="space-y-1.5">
                            <Label htmlFor="exif-iso">ISO speed</Label>
                            <Input
                                id="exif-iso"
                                type="number"
                                step="1"
                                placeholder="e.g. 400"
                                value={fields.iso_speed}
                                onChange={(e) => setField('iso_speed', e.target.value)}
                            />
                        </div>
                        <div className="space-y-1.5">
                            <Label htmlFor="exif-exp-num">Exposure num</Label>
                            <Input
                                id="exif-exp-num"
                                type="number"
                                step="1"
                                placeholder="e.g. 1"
                                value={fields.exposure_time_num}
                                onChange={(e) => setField('exposure_time_num', e.target.value)}
                            />
                        </div>
                        <div className="space-y-1.5">
                            <Label htmlFor="exif-exp-den">Exposure den</Label>
                            <Input
                                id="exif-exp-den"
                                type="number"
                                step="1"
                                placeholder="e.g. 200"
                                value={fields.exposure_time_den}
                                onChange={(e) => setField('exposure_time_den', e.target.value)}
                            />
                        </div>
                    </div>

                    {/* Submit */}
                    <Button type="submit" className="w-full" disabled={isPending}>
                        {isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin"/>}
                        {syncing ? 'Reconciling file…' : 'Save'}
                    </Button>
                </form>
            </DialogContent>
        </Dialog>
    )
}
