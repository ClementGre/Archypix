#!/usr/bin/env bash
#
# Trip the login rate limiter on an open endpoint (feature 28 §9).
#
# Targets POST /api/auth/login, which is rate-limited per-username at
# RATE_LIMIT_LOGIN_MAX attempts / RATE_LIMIT_LOGIN_WINDOW_SECS (defaults: 10 / 300s).
# The limiter runs *before* credential verification, so wrong passwords still count —
# we just fire the same username repeatedly and watch for the first HTTP 429.
#
# Usage:
#   ./hit_rate_limit.sh                       # defaults: b1.archypix.test, 15 attempts
#   BASE=http://b2.archypix.test ./hit_rate_limit.sh
#   ATTEMPTS=30 USERNAME=alice ./hit_rate_limit.sh
#
set -u

BASE="${BASE:-http://b1.archypix.test}"
USERNAME="${USERNAME:-ratelimit-probe}"
PASSWORD="${PASSWORD:-wrong-password}"
ATTEMPTS="${ATTEMPTS:-15}"
URL="$BASE/api/auth/login"

echo "→ Target:   $URL"
echo "→ Username: $USERNAME  (bucket: login:$USERNAME)"
echo "→ Attempts: $ATTEMPTS"
echo

first_429=""
for i in $(seq 1 "$ATTEMPTS"); do
  # -s silent, -o /dev/null discard body, write out the status code
  code=$(curl -s -o /dev/null -w '%{http_code}' \
    -X POST "$URL" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$USERNAME\",\"password\":\"$PASSWORD\"}")

  case "$code" in
    429) label="RATE LIMITED (429)"; [ -z "$first_429" ] && first_429="$i" ;;
    401) label="rejected creds (401)" ;;
    000) label="NO RESPONSE (is the backend up? DNS/hosts?)" ;;
    *)   label="HTTP $code" ;;
  esac
  printf '  attempt %2d → %s\n' "$i" "$label"
done

echo
if [ -n "$first_429" ]; then
  echo "✓ Rate limit reached — first 429 on attempt #$first_429."
  echo "  (Shows one full 429 response body below.)"
  echo
  curl -s -D - -X POST "$URL" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$USERNAME\",\"password\":\"$PASSWORD\"}"
  echo
  echo "  The rejection is recorded in the 'login' category — check the admin Rate limiting tab."
else
  echo "✗ Never hit 429 in $ATTEMPTS attempts."
  echo "  Either the limit is > $ATTEMPTS, the window reset, or Redis is down (limiter fails open)."
fi
