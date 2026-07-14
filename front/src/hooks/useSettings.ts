import {useMutation, useQuery, useQueryClient} from '@tanstack/react-query'
import {getSettings, getStorage, updateProfile, updateSettings} from '@/api/settings'
import {queryKeys} from '@/lib/constants'
import {useAuthStore} from '@/stores/auth'

export function useSettings() {
    return useQuery({
        queryKey: queryKeys.settings(),
        queryFn: getSettings,
    })
}

/**
 * The caller's storage quota/usage (feature 22). No polling — refreshed by explicit invalidation
 * (see {@link invalidateStorageDebounced}) after trash/restore/upload.
 */
export function useStorage(enabled = true) {
    return useQuery({
        queryKey: queryKeys.storage(),
        queryFn: getStorage,
        staleTime: 30_000,
        enabled,
    })
}

export function useUpdateSettings() {
    const queryClient = useQueryClient()
    return useMutation({
        mutationFn: updateSettings,
        onSuccess: () => {
            void queryClient.invalidateQueries({queryKey: ['settings']})
        },
    })
}

export function useUpdateProfile() {
    return useMutation({
        mutationFn: updateProfile,
        onSuccess: (profile) => {
            const current = useAuthStore.getState().user
            if (current) {
                useAuthStore.getState().setUser({
                    ...current,
                    display_name: profile.display_name,
                    email: profile.email,
                })
            }
        },
    })
}
