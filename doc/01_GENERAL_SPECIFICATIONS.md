# Full specification

## 1. Core Model: Tags

Tags are hierarchical paths. A picture can carry any number of tags.

```
/Photos/Travel/Alps
/Images/Icons/Profile
/SharedToMe/alice@instance.com/Photos/Travel/Alps
```

**Rules:**

- Tag paths are case-sensitive and slash-delimited. Authorized characters are `[A-Za-z0-9_]` and `/` as a delimiter.
- A tag implicitly includes all its ancestors: assigning `/Photos/Travel/Alps` means the picture also has `/Photos/Travel` and `/Photos`. Ancestor
  tags are virtual — only the explicitly assigned tag is stored; ancestors are derived on read.
- Each stored tag records its **source** (`manual`, `rule`, `segment`, `share_mapping`, `incoming_share`). The same path may be asserted independently
  by several sources on one picture (e.g. a manual tag plus a rule that also matches), so storage is keyed per-source rather than per-path. Default
  reads fold these to the deepest distinct paths; a provenance view can list every source behind a tag.
- The global unique identifier for a picture is the composite key `(owner, picture\_id)`, where `picture\_id` is unique within an instance. This key
  is used everywhere: tag records, federation messages, WebDAV virtual entries, share announcements.

 --- 

## 2. Deletion and Trash

Deletion is never immediate for received pictures, and is deferred for owned pictures.

| Picture type      | On delete                                  | Physical removal                                                                 |
|:------------------|:-------------------------------------------|:---------------------------------------------------------------------------------|
| Owned             | Marked `deleted\_at = <timestamp>`         | After user-configured retention (e.g. 30 days), permanently deleted from storage |
| Received (shared) | Marked `deleted\_at = <timestamp>` locally | Never physically deleted — the file lives on the sender's storage                |

A deleted picture retains all its tag records internally but is excluded from all views and WebDAV listings. The trash is a separate UI view.
Restoring a picture from trash clears
`deleted\_at`.
--- 

## 3. TaggingServices

A TaggingService assigns tags to pictures according to a rule. Services are ordered into a **pipeline** that runs in sequence. Each service may
declare `requires` and `excludes` lists of tags that gate whether the service fires on a given picture. A service fires only when the picture has *
*all** tags in `requires` and **none** of the tags in `excludes`. Tag presence is evaluated inclusively against virtual ancestors: a picture with
stored tag `/Photos/Travel/Alps` satisfies `requires: [/Photos]`.

### 3.1 Pipeline Execution

The pipeline is event-driven. Each event carries one or more **labels**, and each service declares which labels trigger it. This avoids full pipeline
re-runs on every event.

| Event                                          | Labels              |
|:-----------------------------------------------|:--------------------|
| `IncomingShare` created or updated             | `incoming-share`    |
| New picture ingested (upload or WebDAV)        | `ingest`            |
| Picture metadata edited (EXIF, filename)       | `metadata`          |
| Manual tag assigned or removed                 | `manual-tag`        |
| `RuleTaggingService` definition edited         | `rule-edit`         |
| `SegmentationTaggingService` definition edited | `segmentation-edit` |

Each service declares its trigger labels. When an event fires, only services whose labels intersect the event's labels are re-run, and only on the
affected pictures, in the order given here:

### 3.2 SharedTagMappingService

Operates exclusively on `/SharedToMe/...` tags. Maps pictures received via an `IncomingShare` to local tags, allowing the user to integrate foreign
pictures into their own tag hierarchy.   
**Trigger labels:** `incoming-share`

```
SharedTagMappingService:
  mappings:
    - source: is-001          # IncomingShare id
      assignTag: /Photos/Holidays/2024
    - source: is-003
      assignTag: /Photos/Friends/Bob
```

- Multiple mappings can match a single picture; all matching tags are assigned.
- This service is index-based: it looks up pictures by `IncomingShare` id rather than scanning all tags, making it efficient.
- If the referenced `IncomingShare` is revoked, the mapping produces no pictures and is flagged in the UI.

### 3.3 RuleTaggingService

Assigns tags based on predicates over EXIF fields, filename patterns, GPS bounding boxes, etc.   
**Trigger labels:**  (`incoming-share`), `ingest`, `metadata`, `manual-tag`, `rule-edit`

```
RuleTaggingService:
  rules:
    - predicate: "gps_within_bbox(45.8, 6.8, 46.1, 7.1)"
      assignTag: /Photos/Places/Chamonix
      requires: [/Photos]
```

### 3.4 SegmentationTaggingService

Assigns tags based on capture date ranges.   
**Trigger labels:** (`incoming-share`, `ingest`, `metadata`, `manual-tag`, `rule-edit`), `segmentation-edit`

```
SegmentationTaggingService:
  segments:
    - name: "Alps trip"
      dateRange: [2024-08-01, 2024-08-14]
      assignTag: /Photos/Travel/Alps
      requires: [/Photos]
      excludes: [/Images]
      subSegments:
        - name: "Hiking days"
          dateRange: [2024-08-03, 2024-08-07]
          assignTag: /Photos/Travel/Alps/Hiking
```

- Subsegments inherit the parent's `requires`/ `excludes` and assign their tag in addition to the parent's.
- **Overlap rule:** if a picture falls in two overlapping segments at the same depth, all matching tags are assigned. Overlapping same-depth segments
  emit a validation warning.

### 3.5 Tag removal

Pipeline-assigned tags are **live** — re-derived on every run. Previously-stored `rule`/`segment`/`share_mapping` tags no longer produced are removed
atomically. `manual` and `incoming_share` tags are never touched.

Because tags are stored per-source, dropping one source's deep tag never disturbs another source's row for the same path.

- **Disabling** a service removes its tags; re-enabling re-adds them on the next run.
- **Deleting** a service promotes its tags to `manual` (pre-existing manual tag for the same path wins, redundant row dropped), or removes them —
  controlled by a `promote_tags` flag.

 --- 

## 4. Hierarchies (Bidirectional WebDAV)

A Hierarchy maps a filtered view of the tag graph to a filesystem tree, consumed by two front-ends:
the webapp navigation and (later) WebDAV. It is bidirectional: reads render pictures into directory
paths; writes translate back into tag mutations. It stores **no pictures** — every directory resolves
to a tag-set predicate, so membership, counts, and listings are always derived live.

The `config` is an ordered tree of **nodes**, each rendering to a directory. Three kinds:

- **`mirror`** — dynamically expands the live tag subtree under `tagRoot`. `keepDir` keeps the
  `tagRoot` label as a directory level; `collapsed` subtrees roll their pictures up to the nearest
  enabled ancestor; `exclude` subtrees are removed entirely (pictures + directories).
- **`query`** — an explicit tag predicate; may nest. Effective predicate = own ∧ all ancestors.
  `match: all | any` combines a flat `include` list; `exclude` rejects; `matchUntagged` selects
  pictures with no stored tag of any source. A node is writable iff it declares a `writeBack` op-list.
- **`static`** — a pure container (no predicate, no direct pictures).

```jsonc
{
  "version": 1,
  "safeDeleteMode": "singleBranch",   // singleBranch | fullDelete
  "naming": "original",               // original | date | id
  "writeBack": true,                  // master switch; false ⇒ entire hierarchy read-only
  "nodes": [
    { "id": "n_photos", "kind": "mirror", "name": "Photos", "tagRoot": "Photos",
      "keepDir": false,
      "collapsed": ["Photos.Travel.Alps.Hiking"],
      "exclude":   ["Photos.Outdoor"] },
    { "id": "n_fav", "kind": "query", "name": "Favorites", "match": "all",
      "include": ["Starred"],
      "writeBack": { "onAdd": [{"op":"assign","path":"Starred"}],
                     "onRemove": [{"op":"remove","path":"Starred"}] } }
  ]
}
```

The full data model, validation, read resolver, and write-back semantics are specified in
`doc/features/05_hierarchies.md`. The CRUD + read resolver (navigable `tree`/`browse` endpoints) are
implemented; the **write semantics ship over WebDAV** — each hierarchy mounts at `/webdav/{slug}`
with a per-hierarchy HTTP Basic token, and filesystem operations translate to the tag write-back
described below. See `doc/features/06_webdav.md`.

### 4.1 Read

Each node renders to a directory; a `mirror` node expands into a subtree of directories from the live
tag paths under `tagRoot`. A picture is a **direct file** of directory `D` iff it matches `D` and
matches **none** of `D`'s visible children ("most-specific node wins") — no parent/child duplication.
A picture with stored tag `/Photos/Travel/Alps` appears in `/Travel/Alps` (or `/Photos/Travel/Alps`
if `keepDir` is `true`). Pictures under collapsed tags surface in the nearest enabled ancestor
directory instead of disappearing; pictures under excluded subtrees disappear.

### 4.2 Write semantics

| WebDAV operation                                 | Effect                                                                                       |
|:-------------------------------------------------|:---------------------------------------------------------------------------------------------|
| Move picture from `Travel/` to `Outdoor/`        | Tag `/Photos/Travel` removed, `/Photos/Outdoor` added                                        |
| Copy picture into `Outdoor/`                     | `/Photos/Outdoor` added, original tags kept                                                  |
| Upload new picture into `Travel/Alps/`           | Picture ingested, tag `/Photos/Travel/Alps` assigned, pipeline triggered with label `ingest` |
| Delete picture ( `safeDeleteMode: singleBranch`) | Only the tag for the accessed path is removed; picture survives if it has other tags         |
| Delete picture ( `safeDeleteMode: fullDelete`)   | Picture marked `deleted\_at`, moved to trash                                                 |
| Delete received picture (any mode)               | Picture marked `deleted\_at` locally; never physically deleted                               |
| Rename a directory node                          | Tag renamed cascade — see §6                                                                 |

**Received pictures** (owned by another user) can be moved, copied, and deleted via WebDAV under the same rules as owned pictures, subject to
TaggingService conflict checks (§4.3). Deletion marks them `deleted\_at` locally; the file on the sender's storage is unaffected.

### 4.3 TaggingService Conflict

If a write would contradict an active TaggingService rule (e.g. moving a picture out of a segment-assigned tag while the segment still covers its
capture date), the server returns
`409 Conflict` with a human-readable reason identifying the conflicting service and rule.
--- 

## 5. Federation

User identities take the form `@username:instance.com`. Each instance is an independently deployed backend. The Resolver maps usernames to backend
domains via WebFinger.

### 5.1 Components

- **Resolver** — WebFinger endpoint mapping `@user:instance.com` → backend domain. Backed by Postgres with an in-process TTL cache.
- **Backend** — authoritative per-instance server. Owns metadata in Postgres. Serves HTTP API and WebDAV. Handles federation messages. Enqueues jobs
  to a Postgres-backed queue; consumes results from workers via HTTP. Caches hot data in Redis.
- **Workers** — stateless Rust processes. Poll the backend for jobs (thumbnails, EXIF extraction) via HTTP. Publish results back. Never write to the
  database directly; access S3 only via presigned URLs.
- **S3/MinIO** — durable blob store for originals, derivatives, and version snapshots.
- **Frontend** — static CDN. Resolves `@username:instance.com` → backend URL via WebFinger. All API and WebDAV calls go to the resolved backend.

### 5.2 Cross-Instance Picture Fetching

When a client needs to display a picture owned by `@alice:instance.com`, it resolves the backend domain via WebFinger (using the picture's
`owner` field), then fetches the blob directly from that backend via presigned URL. The relaying user's backend is never in the data path — it handles
only metadata: tag assignments, share announcements, and revocations.
--- 

## 6. Sharing

### 6.1 Data model

Sharing is represented by two paired records living on different backends:

- `**OutgoingShare**` — lives on the sender's backend. Declares what is shared, to whom, and under what conditions.
- `**IncomingShare**` — lives on the recipient's backend. Records what was received, from whom, and links to the `SharedTagMappingService` mapping if
  one exists.

```
OutgoingShare:
  id: os-001
  owner: "@alice:instance.com"
  tag: /Photos/Travel/Alps
  name: "Alps 2024"          # required short label shown to both parties
  message: "Hope you enjoy"  # optional free-text note (nullable)
  recipient: "@bob:other.com"
  allowShareBack: true         # if false, ShareBack creates a normal share request (no auto-accept)
  future: true                 # new pictures added to the tag are announced automatically
  sharebackOf: null            # if set, the original OutgoingShare this share answers (ShareBack provenance)
  status: pending              # pending | pending_first_announcement | active | errored | revoked
                               # pending_first_announcement: accepted; the pipeline announces the
                               # current pictures (ignoring `future`) then flips it to active.
                               # errored: a delivery failed; the pipeline retries a full reconcile
                               # (after a backoff) and flips back to active once fully delivered.

IncomingShare:
  id: is-001
  sender: "@alice:instance.com"
  name: "Alps 2024"            # propagated from the sender's OutgoingShare
  message: "Hope you enjoy"    # propagated (nullable)
  outgoingShareId: os-001      # reference to the sender's OutgoingShare
  future: true                 # propagated: whether the sender auto-announces new pictures
  sharedTagPath: /SharedToMe/alice_AT_instance_DOT_com/Photos/Travel/Alps
                               # advisory local tag the pictures land under; set at creation,
                               # refreshed on each announcement (reflects a sender-side tag rename)
  lastAnnouncementReceivedAt: null  # timestamp of the sender's last picture announcement
  localMappingServiceId: stms-007   # optional: linked SharedTagMappingService entry
  sharebackOf: null            # if set, the recipient's own OutgoingShare this is a share-back of
  status: pending              # pending | active | revoked | tombstoned
```

### 6.2 Sharing a tag

Alice shares `/Photos/Travel/Alps` with Bob → `OutgoingShare` created, `IncomingShare` (`pending`) on Bob's backend. Bob accepts → `IncomingShare` →
`active`; pictures registered as `/SharedToMe/alice_AT_instance_DOT_com/Photos/Travel/Alps`. The picture's `owner` remains `@alice:instance.com`;
Bob's client fetches blobs directly from Alice's backend. With `future: true`, new pictures Alice adds are announced automatically (if share is
`active`), re-running Bob's pipeline with label `incoming-share`.

### 6.3 Re-tagging received pictures

Bob can assign local tags to received pictures. `SharedTagMappingService` maps `IncomingShare` `is-001` to `/Photos/Holidays/2024`.
`SegmentationTaggingService` can fire on received pictures — `requires`/`excludes` evaluate against Bob's local tag set (including `/SharedToMe/...`).
None of this mutates Alice's tags or metadata.

### 6.4 Transitive sharing

Bob shares `/Photos/Holidays/2024` (containing both his own and Alice's pictures) to Carol. Carol's backend assigns
`/SharedToMe/bob@other.com/Photos/Holidays/2024`. When Alice adds a picture, the announcement chain propagates: Alice → Bob → Carol. File fetching
always goes directly to the owning backend — Bob is never in the data path.

### 6.5 ShareBack

`allowShareBack: true` on Alice's `OutgoingShare` → Bob's ShareBack auto-accepts on Alice's backend: creates `IncomingShare` +
`SharedTagMappingService` mapping automatically.   
`allowShareBack: false` → normal share request requiring manual acceptance; no automatic mapping.

### 6.6 Loop prevention

Before announcing, Bob's backend checks whether the picture's `owner` matches the recipient — if so, suppresses the announcement (prevents Alice's
pictures relayed through Bob from being re-announced back to Alice). Duplicate detection also prevents loops when Alice shares to both Bob and Carol
and Bob shares transitively to Carol.

### 6.7 Revocation

Alice revokes via `POST /api/authenticated/shares/outgoing/{id}/revoke`. Her backend:

1. Sets `OutgoingShare` to `revoked`.
2. Notifies the recipient (same-backend: directly; cross-instance: `POST /api/federation/shares/revoke`).

Recipient backend on revocation: removes all `/SharedToMe/alice@instance.com/...` tags, deletes received-picture rows with no other active share, sets
`IncomingShare` to
`revoked`, invalidates Redis presign-token cache, propagates revocation downstream to transitive recipients. Bob's own pictures and the tag itself are
unaffected; the broken
`SharedTagMappingService` mapping is flagged in the UI.
--- 

## 7. Important Edge Cases

**Tag rename cascade.** Renaming a tag must update: all stored tag records on affected pictures, segment/hierarchy/share configurations. Must be an
async job (via `TaskQueue::TagRename`), not a synchronous API call.

**Dumb WebDAV client behaviour.** Clients (e.g. Cyberduck, rclone) that see a picture in multiple paths may delete it from all locations on a local
delete. `safeDeleteMode: singleBranch` mitigates this — recommended default for hierarchies used with third-party sync clients.

**Offline sender availability.** If Alice's backend is offline, Bob and Carol cannot fetch her pictures — inherent limitation of the decentralised
model, no special handling needed.
