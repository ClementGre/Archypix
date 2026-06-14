/// <reference types="vite/client" />

interface ImportMetaEnv {
    readonly VITE_GLOBAL_DOMAIN?: string
    readonly VITE_USE_HTTPS?: string
    readonly VITE_REGISTRATION_MODE?: string
    readonly VITE_REGISTRATION_URL?: string
}

interface ImportMeta {
    readonly env: ImportMetaEnv
}
