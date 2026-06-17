import {useCallback, useRef, useState} from 'react'
import {CloudUpload} from 'lucide-react'
import {LeftPanel} from '@/components/layout/LeftPanel'
import {SidePanel} from '@/components/layout/SidePanel'
import {PhotoGrid} from '@/components/photos/PhotoGrid'
import {SelectionPanel} from '@/components/photos/SelectionPanel'
import {HierarchyEditor} from '@/components/hierarchies/HierarchyEditor'
import {useUIStore} from '@/stores/ui'
import {useSelectionStore} from '@/stores/selection'
import {useUploadStore} from '@/stores/upload'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {useIsMobile} from '@/hooks/useMediaQuery'
import {cn} from '@/lib/utils'

/**
 * The main workspace: three panes (left panel, photo grid, selection panel) plus a
 * full-page drag-over zone that opens the upload dialog when files are dropped.
 */
export default function GalleryPage() {
    const {
        leftSidebarOpen,
        rightSidebarOpen,
        leftSidebarWidth,
        rightSidebarWidth,
        mobileDrawer,
        closeMobileDrawer,
        setLeftWidth,
        setRightWidth,
    } = useUIStore()
    const isMobile = useIsMobile()
    const hasSelection = useSelectionStore((s) => s.selected.length > 0)
    const openUpload = useUploadStore((s) => s.openDialog)
    const {params} = useGalleryParams()
    const editingHierarchy = params.hedit

    // Desktop docks the panels (persisted toggle); mobile shows them as one-at-a-time
    // overlay drawers driven by the session-only `mobileDrawer` state.
    const leftOpen = isMobile ? mobileDrawer === 'left' : leftSidebarOpen
    const rightOpen = isMobile ? mobileDrawer === 'right' : rightSidebarOpen

    const [dragOver, setDragOver] = useState(false)
    const dragCounter = useRef(0)

    const onDragEnter = useCallback((e: React.DragEvent) => {
        e.preventDefault()
        if (!e.dataTransfer.types.includes('Files')) return
        dragCounter.current++
        setDragOver(true)
    }, [])

    const onDragLeave = useCallback(() => {
        dragCounter.current--
        if (dragCounter.current === 0) setDragOver(false)
    }, [])

    const onDragOver = useCallback((e: React.DragEvent) => {
        e.preventDefault()
    }, [])

    const onDrop = useCallback(
        (e: React.DragEvent) => {
            e.preventDefault()
            dragCounter.current = 0
            setDragOver(false)
            if (e.dataTransfer.files.length > 0) {
                openUpload(Array.from(e.dataTransfer.files))
            }
        },
        [openUpload],
    )

    return (
        <div
            className="relative flex h-full min-h-0"
            onDragEnter={onDragEnter}
            onDragLeave={onDragLeave}
            onDragOver={onDragOver}
            onDrop={onDrop}
        >
            <SidePanel
                side="left"
                width={leftSidebarWidth}
                onResize={setLeftWidth}
                open={leftOpen}
                onClose={closeMobileDrawer}
            >
                <LeftPanel/>
            </SidePanel>

            <div className="min-w-0 flex-1">
                {editingHierarchy ? <HierarchyEditor id={editingHierarchy}/> : <PhotoGrid/>}
            </div>

            {!editingHierarchy && hasSelection && (
                <SidePanel
                    side="right"
                    width={rightSidebarWidth}
                    onResize={setRightWidth}
                    open={rightOpen}
                    onClose={closeMobileDrawer}
                >
                    <SelectionPanel/>
                </SidePanel>
            )}

            {/* Drop overlay */}
            {dragOver && (
                <div className={cn(
                    'pointer-events-none absolute inset-0 z-40 flex flex-col items-center justify-center gap-4',
                    'rounded-sm border-2 border-dashed border-primary bg-primary/10 backdrop-blur-sm',
                )}>
                    <CloudUpload className="h-14 w-14 text-primary"/>
                    <p className="text-lg font-semibold text-primary">Drop photos to upload</p>
                </div>
            )}
        </div>
    )
}
