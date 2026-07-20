# Archypix Frontend

**Web client for [Archypix](../README.md)** — a single-page React app for browsing, tagging, sharing, and organizing a federated photo library.

## Overview

The frontend is a pure static SPA (no SSR). It is **federated**: rather than talking to one fixed API server, it resolves — per logged-in user — which
backend hosts that user's `@username:domain` identity via the resolver (`/archypix-resolver/resolve`), then talks directly to that backend. Picture
files are fetched straight from the
owner's object storage through short-lived presigned URLs, so large file traffic never passes through a relay.

The whole client lives in `src/` (component source included, not hidden in `node_modules`) so it is easy to read, grep, and edit.
See [doc/05_FRONTEND_ARCHITECTURE.md](../doc/05_FRONTEND_ARCHITECTURE.md) for the full architecture and conventions.

## Stack

| Concern      | Choice                                 |
|--------------|----------------------------------------|
| UI           | React 19 + TypeScript                  |
| Build        | Vite                                   |
| Styling      | Tailwind CSS v4 + shadcn/ui (zinc/sky) |
| Routing      | React Router v7                        |
| Server state | TanStack Query v5                      |
| Client state | Zustand                                |
| Forms        | React Hook Form + Zod                  |
| HTTP         | axios (JWT attach + auto-refresh)      |
| Misc         | @dnd-kit, blurhash, sonner, lucide     |

## Getting started

Prerequisites: **Node 24+** and **npm** (pnpm is not required; the lockfile is `pnpm-lock.yaml` but `npm` works).

```bash
cd front
npm install
npm run dev      # Vite dev server on http://localhost:5173
npm run build    # tsc -b && vite build → dist/
```

`dist/` is a static bundle servable from any CDN; the host must serve `index.html` for all routes (SPA fallback).

## Configuration

Dev values are read from `front/.env` (see [.env.example](.env.example)). All vars are build-time (`VITE_` prefixed):

| Variable                 | Default         | Purpose                                                                                             |
|--------------------------|-----------------|-----------------------------------------------------------------------------------------------------|
| `VITE_GLOBAL_DOMAIN`     | `archypix.test` | Default identity domain — the part after `:` in `@user:domain`. Used for resolution + registration. |
| `VITE_USE_HTTPS`         | `false`         | Scheme used to reach the global domain (and resolved backends). `false` → http (dev).               |
| `VITE_REGISTRATION_MODE` | `auto`          | `auto` (try resolver, fall back to standalone), `resolver`, or `standalone`.                        |
| `VITE_REGISTRATION_URL`  | *(empty)*       | Explicit registration endpoint override; when set, used verbatim.                                   |

## Local development against the dev stack

The repo ships a federation stack in [`docker/docker-compose.dev.yml`](../docker/docker-compose.dev.yml), fronted by Traefik on port 80:

| Host                 | Service                                                                        |
|----------------------|--------------------------------------------------------------------------------|
| `archypix.test`      | Resolver (`/archypix-resolver/resolve` + `/api/public/register`)               |
| `b1.archypix.test`   | Backend 1 (resolver-backed)                                                    |
| `b2.archypix.test`   | Backend 2 (resolver-backed)                                                    |
| `solo.archypix.test` | Standalone backend (own `/archypix-resolver/resolve` + `/api/public/register`) |

Add the fake hostnames to `/etc/hosts`, then start the stack:

```bash
sudo tee -a /etc/hosts < ../docker/hosts
docker compose -f ../docker/docker-compose.dev.yml up --build
npm run dev
```

With `VITE_GLOBAL_DOMAIN=archypix.test`, **log in** as an existing `@user:archypix.test` (the resolver resolves to `b1`/`b2`) — or switch the instance
on
the login form to `solo.archypix.test` for the standalone backend. **Register** creates a user on the global domain (resolver or standalone,
auto-detected). Backends run with `CORS_ORIGINS=*` in dev, so the cross-origin direct-to-backend calls work from `localhost:5173`.

## License

Archypix is released under the [GNU AGPL v3.0](../LICENSE).
