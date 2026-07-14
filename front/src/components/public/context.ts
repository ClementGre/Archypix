import {createContext, useContext} from 'react'
import type {PublicShareMeta} from '@/api/publicShares'

/** Everything the public-share components need to talk to the owner backend. */
export interface PublicShareContext {
    backendUrl: string
    token: string
    ownerUsername: string
    globalDomain: string
    meta: PublicShareMeta
    /** Unlock JWT for a password-gated share, else `null`. */
    session: string | null
}

const Ctx = createContext<PublicShareContext | null>(null)

export const PublicShareProvider = Ctx.Provider

export function usePublicShare(): PublicShareContext {
    const ctx = useContext(Ctx)
    if (!ctx) throw new Error('usePublicShare must be used within a PublicShareProvider')
    return ctx
}
