# Photos fix tools

> **Superseded.** This rough stub was split into two settled specs:
> [29_query_proximity_and_missing_filter.md](29_query_proximity_and_missing_filter.md)
> (the reusable `missing` filter + time/geo proximity sorts) and
> [30_photos_fix_tools.md](30_photos_fix_tools.md) (the GPS/date fix modes). Kept for the
> decision trail; do not implement from this file.

These modes should be available through a tool button in the breadcumb, allowing to enable the corresponding mode.

## GPS FIX

Sets a picture GPS location based on the picture after and before in time. Should be a special tool in the frontend that highlights in the grid the
pictures without GPS. When a picture is clicked, it should show on a map the three points of the two nearest pictures, and show a weighted average of
the two points for the picture wtihout GPS. Should have a clean UI allowing both to check that the average location is right, allowing to edit it, and
allowing to be efficient when fixing a lot of pictures.

## Capture date fix

In this mode, pictures with no capture date should be highlighted, and when clicked, a date should be suggested automatically based on the picture
name or added at date.
