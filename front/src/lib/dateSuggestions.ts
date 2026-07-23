// Capture-date suggestions for a picture, in priority order (feature 30 §6): filename guess (best),
// its swapped-order alternative when ambiguous, the source file date, then the upload date. Shared by
// the DateFixPanel chip row and the normal EXIF date editor's "From …" prefills.

import {parseFilenameDate} from '@/lib/filenameDate'
import {toNaive} from '@/lib/fixDate'

export type DateSuggestionKey = 'filename' | 'filename-alt' | 'source' | 'uploaded'

export interface DateSuggestion {
    key: DateSuggestionKey
    /** Short "From …" label for the chip. */
    label: string
    /** NaiveDateTime "YYYY-MM-DDTHH:MM:SS". */
    value: string
    lowConfidence?: boolean
}

export function dateSuggestions(picture: {
    filename: string | null
    original_file_created_at: string | null
    ingested_at: string
}): DateSuggestion[] {
    const out: DateSuggestion[] = []
    const fromName = parseFilenameDate(picture.filename)
    if (fromName) {
        out.push({key: 'filename', label: 'From filename', value: fromName.value, lowConfidence: fromName.confidence === 'low'})
        if (fromName.alternative) {
            out.push({key: 'filename-alt', label: 'Swapped day/month', value: fromName.alternative.value, lowConfidence: true})
        }
    }
    const src = toNaive(picture.original_file_created_at)
    if (src) out.push({key: 'source', label: 'From file date', value: src, lowConfidence: true})
    const up = toNaive(picture.ingested_at)
    if (up) out.push({key: 'uploaded', label: 'From upload', value: up})
    return out
}
