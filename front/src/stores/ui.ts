import {create} from 'zustand'

const LS_KEY = 'archypix_ui'

interface Persisted {
  leftSidebarOpen: boolean
  rightSidebarOpen: boolean
  /** Baseline row height (px) for the justified photo grid; drives flex-basis. */
  rowHeight: number
  /** Show tag provenance (sources) instead of a plain tag list in the details panel. */
  tagProvenance: boolean
}

const DEFAULTS: Persisted = {
  leftSidebarOpen: true,
  rightSidebarOpen: true,
  rowHeight: 200,
  tagProvenance: false,
}

export const ROW_HEIGHT_MIN = 120
export const ROW_HEIGHT_MAX = 380

function load(): Persisted {
  try {
    const raw = localStorage.getItem(LS_KEY)
    if (raw) return {...DEFAULTS, ...(JSON.parse(raw) as Partial<Persisted>)}
  } catch {
    // ignore
  }
  return DEFAULTS
}

interface UIState extends Persisted {
  toggleLeft: () => void
  toggleRight: () => void
  setRowHeight: (rowHeight: number) => void
  toggleTagProvenance: () => void
}

function save(state: Persisted) {
  localStorage.setItem(
      LS_KEY,
      JSON.stringify({
        leftSidebarOpen: state.leftSidebarOpen,
        rightSidebarOpen: state.rightSidebarOpen,
        rowHeight: state.rowHeight,
        tagProvenance: state.tagProvenance,
      }),
  )
}

export const useUIStore = create<UIState>((set, get) => ({
  ...load(),
  toggleLeft: () => {
    set({leftSidebarOpen: !get().leftSidebarOpen})
    save(get())
  },
  toggleRight: () => {
    set({rightSidebarOpen: !get().rightSidebarOpen})
    save(get())
  },
  setRowHeight: (rowHeight) => {
    set({rowHeight})
    save(get())
  },
  toggleTagProvenance: () => {
    set({tagProvenance: !get().tagProvenance})
    save(get())
  },
}))
