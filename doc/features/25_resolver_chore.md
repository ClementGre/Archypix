- Show fleet dashboard access buttons only if the instance of the user uses a resolver
- connect to the resolver of the user’s global domain if authenticated, otherwise
- When registering, allow to edit the global domain even if registration is closed.
- When connecting to the resolver, allow to specify a different resolver domain than the one of the authenticated user, fallbacked to the instance
  default global domain (env configured).
- resolver-api prefix (resolved): drop `.well-known/webfinger` entirely — it's not RFC-compliant here anyway, and reusing that exact path risks
  colliding with real WebFinger/Matrix discovery if a self-hoster also runs Mastodon/Matrix on the same apex domain. Instead, put everything —
  identity resolution, registration routing, and the dashboard — under one custom prefix, `/archypix-resolver-api/`. A self-hoster then has exactly
  one forwarding rule to configure, no `.well-known` special-casing, no conflict with other apps on the domain.
- no-resolver / different-host bootstrap (resolved): add `GET /archypix-resolver-api/info`, served by whatever answers the global domain root,
  always the same shape:
  ```json
  { "is_resolver": false, "api_url": "https://archypix-resolver.example.com" }
  ```
  or, when a resolver exists:
  ```json
  { "is_resolver": true, "api_url": "https://arphypix.example.com" }
  ```
  The frontend calls this once against the global domain before anything else. If `is_resolver` is false, it talks to `api_url` directly for
  everything (register/login/API) and skips user resolution entirely (single backend, nothing to resolve). If true, it uses `api_url` as the
  resolver base for `GET /archypix-resolver-api/resolve?user={user}&domain={domain}` (replaces the old `resource=archypix:@user` webfinger query)
  and for `/api/public/register` / dashboard routing. This shrinks the self-hoster's manual step from "reverse-proxy the whole `/api` surface" to
  "serve one static JSON file at one fixed path."
