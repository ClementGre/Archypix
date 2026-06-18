/// <reference types="vite/client" />

interface ImportMetaEnv {
    readonly VITE_GLOBAL_DOMAIN?: string
    readonly VITE_USE_HTTPS?: string
}

interface ImportMeta {
    readonly env: ImportMetaEnv
}
