// Picks a lucide icon for a file with no usable thumbnail (PDFs, videos, audio, archives, …),
// from its MIME type when known, else its filename extension. Icons come from lucide-react (already
// the app's icon set) — no extra assets.

import {
    File,
    FileArchive,
    FileAudio,
    FileCode,
    FileImage,
    type LucideIcon,
    FileSpreadsheet,
    FileText,
    FileVideo,
} from 'lucide-react'

const EXT_GROUPS: Record<string, string[]> = {
    image: ['jpg', 'jpeg', 'png', 'gif', 'webp', 'heic', 'heif', 'avif', 'bmp', 'tif', 'tiff', 'svg'],
    video: ['mp4', 'mov', 'avi', 'mkv', 'webm', 'm4v', 'mpg', 'mpeg', 'wmv', 'flv'],
    audio: ['mp3', 'wav', 'flac', 'aac', 'ogg', 'oga', 'm4a', 'opus'],
    archive: ['zip', 'rar', '7z', 'tar', 'gz', 'bz2', 'xz'],
    code: ['json', 'xml', 'yaml', 'yml', 'html', 'htm', 'css', 'js', 'ts', 'tsx', 'jsx', 'sh', 'rs', 'py'],
    sheet: ['csv', 'tsv', 'xls', 'xlsx', 'ods'],
    doc: ['pdf', 'txt', 'md', 'rtf', 'doc', 'docx', 'odt'],
}

const ICON_BY_GROUP: Record<string, LucideIcon> = {
    image: FileImage,
    video: FileVideo,
    audio: FileAudio,
    archive: FileArchive,
    code: FileCode,
    sheet: FileSpreadsheet,
    doc: FileText,
}

function groupFromMime(mime: string): string | null {
    if (mime.startsWith('image/')) return 'image'
    if (mime.startsWith('video/')) return 'video'
    if (mime.startsWith('audio/')) return 'audio'
    if (mime.startsWith('text/')) return 'doc'
    if (mime === 'application/pdf') return 'doc'
    if (/(zip|x-tar|x-7z|x-rar|gzip|bzip2)/.test(mime)) return 'archive'
    if (/(json|xml|javascript|yaml|html)/.test(mime)) return 'code'
    if (/(csv|spreadsheet|excel)/.test(mime)) return 'sheet'
    return null
}

function pickIcon(mime?: string | null, filename?: string | null): LucideIcon {
    const g = mime ? groupFromMime(mime.toLowerCase()) : null
    if (g) return ICON_BY_GROUP[g]
    const ext = (filename ?? '').toLowerCase().split('.').pop() ?? ''
    for (const [group, exts] of Object.entries(EXT_GROUPS)) {
        if (exts.includes(ext)) return ICON_BY_GROUP[group]
    }
    return File
}

export function FileTypeIcon({
                                 mime,
                                 filename,
                                 className,
                             }: {
    mime?: string | null
    filename?: string | null
    className?: string
}) {
    const Icon = pickIcon(mime, filename)
    return <Icon className={className}/>
}
