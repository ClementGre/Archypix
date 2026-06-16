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
    - predicate: "exif.gps within bbox(45.8, 6.8, 46.1, 7.1)"
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

Pipeline-assigned tags are **live**: they are a pure function of the current services and picture state, re-derived on every run. When a picture is
re-evaluated, each enabled service produces its current set of tags; any previously-stored `rule`/`segment`/`share_mapping` tag that is no longer
produced is removed in the same atomic step. `manual` and `incoming_share` tags are never touched by the pipeline — the former are owned by the user,
the latter by share accept/revoke.

Because tags are stored per-source, removal needs no special ancestor handling: each source owns its own rows, so dropping one source's deep tag never
disturbs another source's ancestor tag.

Service lifecycle interacts with removal explicitly:

- **Disabling** a service removes its tags (they are no longer live); re-enabling re-adds them on the next run.
- **Deleting** a service **promotes** its tags to `manual`, preserving the curation work as permanent user tags (a manual tag that already exists for
  the same path wins, and the redundant pipeline row is dropped).

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
implemented; the **write endpoints ship with WebDAV**.

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

- **Resolver** — WebFinger endpoint. Maps `@user:instance.com` → backend domain. Backed by Postgres with an in-process TTL cache (moka). Exposes an
  admin API for backends to register users.
- **Backend** — authoritative per-instance application server. Owns user metadata in Postgres. Serves HTTP API and WebDAV. Handles inbound/outbound
  federation messages. Produces jobs to NATS JetStream; consumes results from workers. Optionally caches hot data in Redis.
- **Workers** — central pool of Rust processes. Consume jobs from JetStream (thumbnails, ML inference, face detection, geo clustering). Publish
  compact results back to the owning backend. Never write directly to any backend's database.
- **S3/MinIO** — durable blob store for originals, derivatives, per-user ML snapshots, and exports. Workers access blobs via presigned URLs or scoped
  credentials.
- **Frontend** — static CDN. Resolves `@username:instance.com` → backend domain via WebFinger. All API and WebDAV calls go to the resolved backend.

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
  recipient: "@bob:other.com"
  allowShareBack: true         # if false, ShareBack creates a normal share request (no auto-accept)
  future: true                 # new pictures added to the tag are announced automatically
  status: pending              # pending | pending_first_announcement | active | errored | revoked
                               # pending_first_announcement: accepted; the pipeline announces the
                               # current pictures (ignoring `future`) then flips it to active.
                               # errored: a delivery failed; the pipeline retries a full reconcile
                               # (after a backoff) and flips back to active once fully delivered.

IncomingShare:
  id: is-001
  sender: "@alice:instance.com"
  outgoingShareId: os-001      # reference to the sender's OutgoingShare
  localMappingServiceId: stms-007   # optional: linked SharedTagMappingService entry
  status: pending              # pending | active | revoked | tombstoned
```

### 6.2 Sharing a tag

Alice shares `/Photos/Travel/Alps` with Bob. Alice's backend creates an `OutgoingShare` and federates a share announcement to Bob's backend. Bob's
backend creates an `IncomingShare` (`status = pending`).

Bob then accepts the share. His backend transitions the `IncomingShare` to `active` and registers each picture under the tag:
`/SharedToMe/alice_AT_instance_DOT_com/Photos/Travel/Alps`

The picture's `owner` field remains `@alice:instance.com`. When Bob's client displays it, it fetches the blob directly from Alice's backend.   
If `future: true`, any picture Alice subsequently adds to `/Photos/Travel/Alps` triggers a new announcement to Bob's backend (only delivered if the
share is `active`), which assigns the same `/SharedToMe/...` tag and re-runs Bob's pipeline with label `incoming-share`.

### 6.3 Re-tagging received pictures

Bob can assign any local tags to received pictures. His `SharedTagMappingService` can map the `IncomingShare` `is-001` to `/Photos/Holidays/2024`. His
`SegmentationTaggingService` can also fire on received pictures — `requires`/ `excludes` evaluate against Bob's local tag set, which includes the
`/SharedToMe/...` tags assigned by the `IncomingShare`. None of this mutates Alice's tags or metadata.

### 6.4 Transitive sharing

Bob shares tag `/Photos/Holidays/2024` to Carol. This tag contains both Bob's own pictures and Alice's pictures (mapped via
`SharedTagMappingService`). All pictures under the tag are shared regardless of original owner.   
Carol's backend assigns:

```
/SharedToMe/bob@other.com/Photos/Holidays/2024
```

**Announcement chain:** when Alice adds a picture to `/Photos/Travel/Alps`, her backend notifies Bob's backend. Bob's backend maps it into
`/Photos/Holidays/2024` via `SharedTagMappingService`, and if that tag is covered by a Bob→Carol `OutgoingShare`, Bob's backend announces it to Carol'
s backend.   
**File fetching:** Carol's client fetches Alice's pictures directly from `@alice:instance.com`, resolved via WebFinger from the picture's `owner`.
Bob's backend is not in the data path.

### 6.5 ShareBack

If `allowShareBack: true` on Alice's `OutgoingShare`, Bob can initiate a ShareBack. This creates a normal `OutgoingShare` from Bob to Alice, which
Alice's backend **auto-accepts**: it creates an `IncomingShare` and automatically sets up a `SharedTagMappingService` mapping for it.   
If `allowShareBack: false`, Bob can still initiate a share to Alice, but it is treated as a normal share request — Alice receives a notification and
must accept manually. No automatic `SharedTagMappingService` is created.

### 6.6 Loop prevention

When Bob's backend is about to announce a picture to a recipient, it checks whether the picture's `owner` matches the recipient's identity. If so, the
announcement is suppressed. This covers the case where Alice's pictures, relayed through Bob, would otherwise be re-announced back to Alice.   
Deduplication of incoming pictures is also done automatically to prevent share loops within three users. If Alice shares to Bob and Bob shares
transitively to Calol. If Alice also shares directly to Carol, Carol will deduplicate the received pictures, noticing that the pictures received by
Alice are already shared by Bob.

### 6.7 Revocation

Alice calls `POST /api/authenticated/shares/outgoing/{id}/revoke`. Alice's backend:

1. Sets `OutgoingShare` os-001 status to `revoked`.
2. Notifies the recipient (same-backend: directly; cross-instance: `POST /api/federation/shares/revoke`).

Bob's backend on receiving revocation:

1. Removes all `/SharedToMe/alice@instance.com/...` tag entries assigned by this share.
2. Deletes any received-picture rows from Alice that have no other active incoming-share tag (pictures covered by a separate active share from Alice
   survive).
3. Sets `IncomingShare` is-001 status to `revoked` and invalidates the cached share token so presign requests fail immediately.
4. Propagates revocation downstream to any transitive recipients (Carol, etc.) whose `OutgoingShare`s depended on pictures sourced from is-001.

Bob's own pictures in `/Photos/Holidays/2024` are unaffected. The `SharedTagMappingService` mapping for is-001 is flagged as broken in the UI. The tag
`/Photos/Holidays/2024` remains, possibly now containing fewer pictures, until Bob cleans it up.
--- 

## 7. Important Edge Cases

**Tag rename cascade cost.** Renaming `/Photos/Travel` must update every stored tag record on affected pictures, every segment definition, every
ShareBack trigger, every `OutgoingShare` tag field, and every Hierarchy `roots`/ `disabledTags` config on the instance. This must be an async
transactional job, not a synchronous API call. Clients treat the rename as pending until the job completes and must not allow conflicting writes
during that window.   
**Dumb WebDAV client behaviour.** Standard sync clients (e.g. Cyberduck, rclone) that see a picture in multiple paths (possible if the same picture
has multiple tags under a Hierarchy's roots) may delete it from all visible locations on a local delete. `safeDeleteMode: singleBranch` mitigates this
by removing only the accessed path's tag. This flag should be the recommended default for Hierarchies intended for use with third-party sync clients,
and should be documented prominently.   
**Offline sender availability.** If Alice's backend is offline, Bob and Carol cannot fetch her pictures. This is an inherent limitation of the
decentralised model and requires no special handling in the MVP — it should be documented as expected behaviour.   
