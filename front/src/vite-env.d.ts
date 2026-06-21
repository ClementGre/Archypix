/// <reference types="vite/client" />

interface ImportMetaEnv {
    readonly VITE_GLOBAL_DOMAIN?: string
    readonly VITE_USE_HTTPS?: string
}

interface ImportMeta {
    readonly env: ImportMetaEnv
}

// Populated at container startup by docker-entrypoint.sh (see public/env.js),
// letting VITE_* values be overridden at runtime without rebuilding the image.
interface Window {
    __ENV__?: {
        VITE_GLOBAL_DOMAIN?: string
        VITE_USE_HTTPS?: string
    }
}
