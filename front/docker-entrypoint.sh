#!/bin/sh
set -eu

cat > /usr/share/nginx/html/env.js <<EOF
window.__ENV__ = {
    VITE_GLOBAL_DOMAIN: "${VITE_GLOBAL_DOMAIN:-}",
    VITE_USE_HTTPS: "${VITE_USE_HTTPS:-}"
};
EOF

exec "$@"
