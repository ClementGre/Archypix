import {create} from 'zustand'

interface UploadStore {
    open: boolean
    initialFiles: File[]
    openDialog: (files?: File[]) => void
    closeDialog: () => void
}

export const useUploadStore = create<UploadStore>((set) => ({
    open: false,
    initialFiles: [],
    openDialog: (files = []) => set({open: true, initialFiles: files}),
    closeDialog: () => set({open: false, initialFiles: []}),
}))
