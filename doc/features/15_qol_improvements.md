# Quality of life improvements

## Front UX
> **Status (2026-06-25): implemented.** Decisions taken: the details panel is now toggle-driven with
> an empty placeholder (not auto-opened by selection); selection is always cleared on any gallery view
> change.

- Always or never show sidebar: The sidebar that shows only when a picture is selected make all pictures to shift when selecting a picture. Then it should be better to always or never show the sidebar. When no picture is selected, make it show nothing.
- Sidebar picture de-selection when view changes: make it consistent (TODO)
- In the map popup, the clear button is in top right, in place of a usual close popup button. This leads users to clear by error, thinking it will close the popup. Instead, use a real close button, and put the clear button in the bottom of the popup.
- Upload dialog:
  - Retry network errors: add an option to retry network errors
  - Show overall progression outside of the scrollable list of pictures, so we don’t have to scroll all pictures to get to the overall progression. Use a progress bar.
  - Initial loading of pictures is slow on mobile when importing a lot of pictures: When selection 50-1k+ picures from the upload dialog, the phone screen becomes all black for some time (a few seconds), then we see the prepare progress bar, and then we see the pictures. Maybe do minimal initial work and postpone heavy parsing in the preparing step or at upload step. Don’t read all pictures at once, but get them and hash them in batch (we presign the uploads in batch anyway).
- Tags selection popup:
  - Autocomplete tags without fill: It should be able to autocomplete the create new tag field without selecting directly the tag. On keyboard, allow to hit tag to insert the popup currently focused tag into the search/create tag field. And add a button at the right of each tag in the list that autocompletes the clicked tag into the field. That way we can autocomplete /Event to create a new subtag of Event without re-writting event.
  - Proper error when writting an invalid tag: currently when the tag name in the field is invalid (has reserved prefixes or unauthorized characters), it just don’t show the create tag option in the popup. Instead insert in the popup a red warning for reserved prefixes or unauthorised characters that explains why and which character entered is not authorized. When using - or spaces, automatically replaces to _ and show a yellow/orange warning indicating that x was replaced by y. Do the same for . and \ to /, and accentuated letters to non accentuated letters if possible and easy to do. Otherways, it would fall back to the error. 
- Map:
  - Center of circle/rect area can’t be moved by click: clicking on the map should move the center of the rect/circle.
- Tagging pipeline:
  - Query tagging rule form:
    - For strings, the starts with and ends with have no value field (just nothing...) (they do not have the string argument...)
    - Is set / is not set: not really clear. Instead use in the second field directly is set and is not set instead of having to specify is set in the first and then is set or is not set in the second.
    - Edit button not easy to access: Remove the gates from the list view (when we have a lot of services, the list gets too long) and put the edit button more visible.
    - Rename to Tagging services instead of tagging pipeline in the front. Use more user friendly terms where possible.
- Shift click selects LI: on desktop when shift clicking on pictures, it may sometimes select the pictures as text in the browser, which is strange. Set the ul/li not text selectable.
- Proper query invalidation : uploading pictures does not refreshes tags, updating tagging pipeline should invalidate tags, but with a short delay to let the tags update. Overall clicking on a tag or collapsing/expanding a tag should maybe invalidate to keep it up to date.


## Frontend UX that affects a bit of the back
> **Status (2026-06-25): implemented.** Decisions taken: per-tag include / include-exactly / exclude
> via a `…` menu (+ ⌘/Ctrl-click quick include); strict navigation is the per-tag "Include exactly"
> action only (no separate global toggle); the import auto-tag applies to every upload as specced and
> the trashed duplicates are no longer auto-restored (the front offers a restore button). The rule
> `eq_ic` operator is replaced by a per-string-condition `ignore_case` flag (migrated via 0004).

- Tags navigation features:
  - Add strict tag navigation : add a button on tags or a toggle or grouped buttons or something in the sidebar allowing to switch to a strict tag navigation : requiring tag Event requires to have only the tag even and not any descendant. This would require to add the `pub exact: Vec<TagPath>,` param to the list endpoint and insert in to the predicate.
  - mmore features in the tag sidebar: When one tag is selected, when hovering others tags in the sidebar (or an equivalent for mobile), there should appear buttons for requiring and excluding these tags. Then we can easily request to view all pictures with tag x that do not have the tag y, or pictures with tag x and y, etc.
  - Maybe we could add this as three buttons on hover : include, include exactly, and exclude, or if already included, only a de-include button, same for excluded: a de-exclude button. Since this is also a not very usual way of navigating the tag tree, we could add all of these options as a ... right button for each tag, and allow cmd/ctrl + click on desktop for fast include.
- Tag uploaded pictures and don’t undeleted already existing pictures: remove the un-deletion of deleted pictures that already exited when uploaded on the back. When importing pictures, make the back add a tag Uploaded_YYYY_MM_DD_HH_MM (that date should not be the batch presign or complete date: it should be defined by the front and passed to endpoints to have a consistent date across upload). Already existing pictures should have the tag /Uploaded_YYYY_MM_DD_HH_MM/AlreadyExisting and already existing deleted pictures the tag Uploaded_YYYY_MM_DD_HH_MM/AlreadyExisting/Deleted. The backend should report to the front if an already existing picture was deleted or not, so the front can at the end show x pictures out of y already existing ones were deleted. The front should add a button to undelete the pictures with tag Uploaded_YYYY_MM_DD_HH_MM/AlreadyExisting/Deleted, and should give information about how many pictures were tagged what (like - 80 uploaded pictures, tagged Uploaded_YYYY_MM_DD_HH_MM. - 20 already existing pictures, tagged Uploaded_YYYY_MM_DD_HH_MM/AlreadyExisting. - With 10 that were deleted, tagged .... - total 100 pictures.). Something like that.
- In Query tagging rules, instead of having equals and equals ignore case, add all comparaison types with no case (starts with, ends with, contains, equals), and insert a checkbox at the end "ignore case". This will require back to update the data model of rules.

## Strange edge cases that do not occur always
> **Status (2026-06-25): implemented.** Grid dedups infinite-scroll pages by id; the sidebar preview
> clamps + clips its box so 16:9 images can't overflow; WebDAV PUT returns `201 Created` when an
> existing picture is newly added to a directory (gains the tag) and `204 No Content` only on a true
> no-op.

- When scrolling for a long time, some already-seen photos reappears. If they were selected on top, their duplicate is selected too.
- The sidebar image preview is broken on 16:9 images : they take full height and their right part overflows left (not visible).
- Existing picture added in directory behaves strangely (foldersync error) while new uploads are ok. This might be due to the fact of returning StatusCode::NO_CONTENT. When the picture already existed but had not the tag yet (was not in this dir), we should maybe tell the client that is was a normal upload.

## New complex features
- Auto year tagging
- Hierarchies :
  - When the write back master switch is off, all queries should have writeback blocked at off. Writeback option should be available in the advanced features for each node. For queries, it should have all the currently configurable options. For static, it should be disabled with a hover message explaining that static can’t be written into (if subnodes inherit their writeback from the static and not from the root, it could remain configurable), and mirror should have it too, and drop directories should have it on, non-desactivable.   
  - Write back on Untagged pictures query: untagged pictures queries should be able to have writeback enabled
  - Drop directory as a specific node: a dir that accepts any upload but that returns no content in its propfind.
  - Don't refuse the deletion of a directory if this directory is empty or if it is a drop directory node: some webdav clients delete empty dirs and it should not fail. **(Done 2026-06-25:** `Vfs::delete` accepts an empty directory as a no-op success — a hierarchy dir is virtual, so there's nothing to remove — and refuses a non-empty one with `409 Conflict`. A drop-directory node lists empty by design, so the same check will cover it once that node type lands.**)**
