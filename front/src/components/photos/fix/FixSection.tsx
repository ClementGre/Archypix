// The single-picture fix pane (feature 30 §3), inserted into the details panel right before Tags.
// Shown when fix mode is on AND the picture is missing the active field (a photo that already has a
// date/GPS shows no fix options). Non-collapsible — its fixed frame is what justifies the full-bleed
// map / calendar inside. Reference picking is handled inline by the GPS/Date panels themselves.

import {useGalleryParams} from '@/hooks/useGalleryParams'
import {GpsFixPanel} from './GpsFixPanel'
import {DateFixPanel} from './DateFixPanel'
import {FixPane} from './fixShared'
import type {PictureDetail} from '@/lib/types'

export function FixSection({picture, allowExifEdit}: { picture: PictureDetail; allowExifEdit: boolean }) {
    const {params} = useGalleryParams()
    const fix = params.fix
    if (!fix) return null

    const missing = fix === 'gps' ? picture.gps_lat == null || picture.gps_lng == null : picture.captured_at == null
    if (!missing) return null

    return (
        <FixPane title={fix === 'gps' ? 'Fix location' : 'Fix date'}>
            {fix === 'gps'
                ? <GpsFixPanel target={picture} allowExifEdit={allowExifEdit}/>
                : <DateFixPanel target={picture} allowExifEdit={allowExifEdit}/>}
        </FixPane>
    )
}
