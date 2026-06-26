// Helpers for turning a drag-and-drop or folder selection into a flat list of uploadable files,
// recursing into dropped/selected directories and excluding hidden files (dotfiles).

/** A file is hidden when any segment of its path (relative path or name) is dot-prefixed. */
export function isHiddenFile(file: File): boolean {
    const rel = (file as File & { webkitRelativePath?: string }).webkitRelativePath
    const path = rel && rel.length > 0 ? rel : file.name
    return path.split('/').some((seg) => seg.startsWith('.'))
}

function readEntries(reader: FileSystemDirectoryReader): Promise<FileSystemEntry[]> {
    return new Promise((resolve, reject) => reader.readEntries(resolve, reject))
}

function fileOf(entry: FileSystemFileEntry): Promise<File> {
    return new Promise((resolve, reject) => entry.file(resolve, reject))
}

async function collect(entry: FileSystemEntry, out: File[]): Promise<void> {
    // Skip hidden files and directories (.DS_Store, .git, …) — and never descend into them.
    if (entry.name.startsWith('.')) return
    if (entry.isFile) {
        try {
            out.push(await fileOf(entry as FileSystemFileEntry))
        } catch {
            // Unreadable file — skip it rather than aborting the whole drop.
        }
    } else if (entry.isDirectory) {
        const reader = (entry as FileSystemDirectoryEntry).createReader()
        // `readEntries` yields at most ~100 entries per call; loop until it drains.
        for (; ;) {
            const batch = await readEntries(reader).catch(() => [] as FileSystemEntry[])
            if (batch.length === 0) break
            for (const child of batch) await collect(child, out)
        }
    }
}

/**
 * Flatten a drop's `DataTransfer` into a file list, recursing into dropped directories and
 * excluding hidden files/dirs. **Call it synchronously inside the `drop` handler** — it grabs the
 * entry handles up front (the `DataTransferItemList` is cleared once the handler returns) and then
 * resolves asynchronously. Falls back to `dt.files` (still filtering hidden) when the entries API
 * is unavailable.
 */
export async function filesFromDataTransfer(dt: DataTransfer): Promise<File[]> {
    const entries = dt.items
        ? Array.from(dt.items)
            .filter((it) => it.kind === 'file')
            .map((it) => it.webkitGetAsEntry?.() ?? null)
            .filter((e): e is FileSystemEntry => e !== null)
        : []
    if (entries.length === 0) {
        return Array.from(dt.files).filter((f) => !isHiddenFile(f))
    }
    const out: File[] = []
    for (const entry of entries) await collect(entry, out)
    return out
}
