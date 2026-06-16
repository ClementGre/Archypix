import {create} from 'zustand'

const LS_KEY = 'archypix_ui'

interface Persisted {
  leftSidebarOpen: boolean
  rightSidebarOpen: boolean
  /** Width (px) of the left panel; clamped to [SIDEBAR_MIN, SIDEBAR_MAX]. */
  leftSidebarWidth: number
  /** Width (px) of the right (details) panel; clamped to [SIDEBAR_MIN, SIDEBAR_MAX]. */
  rightSidebarWidth: number
  /** Baseline row height (px) for the justified photo grid; drives flex-basis. */
  rowHeight: number
  /** Show tag provenance (sources) instead of a plain tag list in the details panel. */
  tagProvenance: boolean
}

const DEFAULTS: Persisted = {
  leftSidebarOpen: true,
  rightSidebarOpen: true,
  leftSidebarWidth: 256,
  rightSidebarWidth: 288,
  rowHeight: 200,
  tagProvenance: false,
}

export const ROW_HEIGHT_MIN = 120
export const ROW_HEIGHT_MAX = 380

export const SIDEBAR_MIN = 200
export const SIDEBAR_MAX = 520

const clamp = (v: number, min: number, max: number) => Math.min(max, Math.max(min, v))

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
  /**
   * Which side panel is shown as an overlay drawer on mobile. Session-only (never
   * persisted) and independent from the desktop dock open-state above, so opening a
   * drawer on a phone neither auto-shows on load nor clobbers the desktop layout.
   */
  mobileDrawer: 'left' | 'right' | null
  toggleLeft: () => void
  toggleRight: () => void
  setLeftOpen: (open: boolean) => void
  setRightOpen: (open: boolean) => void
  setLeftWidth: (width: number) => void
  setRightWidth: (width: number) => void
  setRowHeight: (rowHeight: number) => void
  toggleTagProvenance: () => void
  toggleMobileDrawer: (side: 'left' | 'right') => void
  closeMobileDrawer: () => void
}

function save(state: Persisted) {
  localStorage.setItem(
      LS_KEY,
      JSON.stringify({
        leftSidebarOpen: state.leftSidebarOpen,
        rightSidebarOpen: state.rightSidebarOpen,
        leftSidebarWidth: state.leftSidebarWidth,
        rightSidebarWidth: state.rightSidebarWidth,
        rowHeight: state.rowHeight,
        tagProvenance: state.tagProvenance,
      }),
  )
}

export const useUIStore = create<UIState>((set, get) => ({
  ...load(),
  mobileDrawer: null,
  toggleMobileDrawer: (side) => set({mobileDrawer: get().mobileDrawer === side ? null : side}),
  closeMobileDrawer: () => set({mobileDrawer: null}),
  toggleLeft: () => {
    set({leftSidebarOpen: !get().leftSidebarOpen})
    save(get())
  },
  toggleRight: () => {
    set({rightSidebarOpen: !get().rightSidebarOpen})
    save(get())
  },
  setLeftOpen: (open) => {
    if (get().leftSidebarOpen === open) return
    set({leftSidebarOpen: open})
    save(get())
  },
  setRightOpen: (open) => {
    if (get().rightSidebarOpen === open) return
    set({rightSidebarOpen: open})
    save(get())
  },
  setLeftWidth: (width) => {
    set({leftSidebarWidth: clamp(width, SIDEBAR_MIN, SIDEBAR_MAX)})
    save(get())
  },
  setRightWidth: (width) => {
    set({rightSidebarWidth: clamp(width, SIDEBAR_MIN, SIDEBAR_MAX)})
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
