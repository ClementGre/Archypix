// Default runtime config, used in dev and as a placeholder in the built image.
// In production, the container entrypoint overwrites this file from environment
// variables at startup (see docker-entrypoint.sh), so values can change without
// rebuilding the image.
window.__ENV__ = {
    VITE_GLOBAL_DOMAIN: "",
    VITE_USE_HTTPS: "",
};
