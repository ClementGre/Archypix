//! HEIF orientation: viewers take it from the primary item's `irot`/`imir` properties and ignore
//! EXIF (feature 37 §3.4). Only the `meta` box is rewritten; the media data is copied untouched.

use crate::error::{Result, WorkerError};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::path::Path;

/// EXIF orientation `i + 1` as (clockwise quarter turns, left-right flip applied first).
const DIHEDRAL: [(u8, bool); 8] = [
    (0, false),
    (0, true),
    (2, false),
    (2, true),
    (3, true),
    (1, false),
    (1, true),
    (3, false),
];

/// What ImageIO writes for EXIF orientation `i + 1`: the `irot` angle (counter-clockwise quarter
/// turns), then the `imir` axis if any (1 = left-right, 0 = top-bottom).
const IMAGEIO: [(u8, Option<u8>); 8] = [
    (0, None),
    (0, Some(1)),
    (2, None),
    (0, Some(0)),
    (1, Some(0)),
    (3, None),
    (1, Some(1)),
    (1, None),
];

/// A HEIF file's `meta` box, parsed down to what orientation needs.
pub struct Heif {
    /// Where `meta` sits in the file.
    span: Range<u64>,
    meta: Vec<u8>,
    /// A `moov` follows `meta`: its chunk offsets would move with a resize.
    moov_after: bool,
    primary: u32,
    /// The primary item and its thumbnails.
    targets: Vec<u32>,
    /// `ipco` children: property `i + 1` is `props[i]`.
    props: Vec<Vec<u8>>,
    ipma: Vec<Entry>,
}

struct Entry {
    item: u32,
    assocs: Vec<Assoc>,
}

struct Assoc {
    index: u16,
    essential: bool,
}

impl Heif {
    /// `Ok(None)` for anything but a HEIF image: a leading `ftyp` and a `meta` with a `pict` handler.
    pub fn open(path: &Path) -> Result<Option<Heif>> {
        let mut file = File::open(path)?;
        let len = file.metadata()?.len();
        let (mut pos, mut span, mut moov_after) = (0u64, None, false);
        while pos < len {
            let mut head = [0u8; 16];
            let n = (len - pos).min(16) as usize;
            file.seek(SeekFrom::Start(pos))?;
            file.read_exact(&mut head[..n])?;
            let Ok((kind, _, size)) = header(&head[..n]) else {
                break;
            };
            if pos == 0 && &kind != b"ftyp" {
                return Ok(None);
            }
            let end = if size == 0 {
                len
            } else {
                pos.saturating_add(size).min(len)
            };
            match &kind {
                b"meta" => span = Some(pos..end),
                b"moov" if span.is_some() => moov_after = true,
                _ => {}
            }
            pos = end;
        }
        let Some(span) = span else {
            return Ok(None);
        };
        let mut meta = vec![0; (span.end - span.start) as usize];
        file.seek(SeekFrom::Start(span.start))?;
        file.read_exact(&mut meta)?;
        Self::parse(span, meta, moov_after)
    }

    fn parse(span: Range<u64>, meta: Vec<u8>, moov_after: bool) -> Result<Option<Heif>> {
        let kids = meta_children(&meta)?;
        let find = |kind: &[u8; 4]| kids.iter().find(|c| &c.kind == kind);
        let handler = |c: &Child| meta.get(c.body() + 8..c.body() + 12);
        if find(b"hdlr").and_then(handler) != Some(&b"pict"[..]) {
            return Ok(None);
        }

        let pitm = find(b"pitm").ok_or_else(|| malformed("no primary item"))?;
        let primary = be(&meta, pitm.body() + 4, id_size(be(&meta, pitm.body(), 1)?))? as u32;
        let mut targets = vec![primary];
        if let Some(iref) = find(b"iref") {
            let width = id_size(be(&meta, iref.body(), 1)?);
            for r in children(&meta, iref.body() + 4..iref.range.end)? {
                if &r.kind != b"thmb" {
                    continue;
                }
                let from = be(&meta, r.body(), width)? as u32;
                let count = be(&meta, r.body() + width, 2)? as usize;
                for i in 0..count {
                    if be(&meta, r.body() + width + 2 + i * width, width)? == u64::from(primary) {
                        targets.push(from);
                    }
                }
            }
        }

        let iprp = find(b"iprp").ok_or_else(|| malformed("no item properties"))?;
        let (mut props, mut ipma) = (Vec::new(), Vec::new());
        for c in children(&meta, iprp.body()..iprp.range.end)? {
            match &c.kind {
                b"ipco" => {
                    props = children(&meta, c.body()..c.range.end)?
                        .into_iter()
                        .map(|p| meta[p.range].to_vec())
                        .collect();
                }
                b"ipma" => parse_ipma(&meta, &c, &mut ipma)?,
                _ => {}
            }
        }
        if ipma
            .iter()
            .flat_map(|e| &e.assocs)
            .any(|a| usize::from(a.index) > props.len())
        {
            return Err(malformed("association to a missing property"));
        }
        if !ipma.iter().any(|e| e.item == primary) {
            return Err(malformed("primary item has no properties"));
        }
        Ok(Some(Heif {
            span,
            meta,
            moov_after,
            primary,
            targets,
            props,
            ipma,
        }))
    }

    /// The primary item's display orientation; `None` without any `irot`/`imir`.
    pub fn orientation(&self) -> Option<i16> {
        let entry = self.ipma.iter().find(|e| e.item == self.primary)?;
        compose(
            entry
                .assocs
                .iter()
                .filter_map(|a| prop(&self.props, a.index)),
        )
    }

    /// Rewrite the file so the primary item and its thumbnails display at `orientation`.
    pub fn write_orientation(mut self, path: &Path, orientation: Option<i16>) -> Result<()> {
        let wanted = match orientation {
            None => vec![],
            Some(o) => {
                let &(angle, axis) = usize::try_from(o - 1)
                    .ok()
                    .and_then(|i| IMAGEIO.get(i))
                    .ok_or_else(|| WorkerError::Exif(format!("invalid orientation {o}")))?;
                let mut wanted = vec![property(b"irot", angle)];
                wanted.extend(axis.map(|axis| property(b"imir", axis)));
                wanted
            }
        };
        let indices: Vec<u16> = wanted
            .into_iter()
            .map(|p| intern(&mut self.props, p))
            .collect();
        for entry in self
            .ipma
            .iter_mut()
            .filter(|e| self.targets.contains(&e.item))
        {
            entry
                .assocs
                .retain(|a| !is_transform(prop(&self.props, a.index)));
            entry.assocs.extend(indices.iter().map(|&index| Assoc {
                index,
                essential: true,
            }));
        }
        drop_unused_transforms(&mut self.props, &mut self.ipma);

        let meta = self.rebuild()?;
        if meta != self.meta {
            splice(path, &self.span, &meta)?;
        }
        Ok(())
    }

    /// The `meta` box with the current properties, its `iloc` following the size change.
    fn rebuild(&self) -> Result<Vec<u8>> {
        let (_, head, _) = header(&self.meta)?;
        let mut body = self
            .meta
            .get(head..head + 4)
            .ok_or_else(|| malformed("truncated meta"))?
            .to_vec();
        let mut iloc = None;
        for c in meta_children(&self.meta)? {
            let bytes = &self.meta[c.range.clone()];
            match &c.kind {
                b"iprp" => body.extend(self.iprp(&c)?),
                b"iloc" => {
                    iloc = Some((body.len(), bytes.len(), c.header));
                    body.extend_from_slice(bytes);
                }
                _ => body.extend_from_slice(bytes),
            }
        }
        let mut meta = boxed(b"meta", &body);
        let delta = meta.len() as i64 - self.meta.len() as i64;
        if delta != 0 && self.moov_after {
            return Err(unsupported("a movie box after meta"));
        }
        if let Some((at, len, iloc_header)) = iloc {
            let at = at + meta.len() - body.len();
            shift_iloc(&mut meta[at..at + len], iloc_header, &self.span, delta)?;
        }
        Ok(meta)
    }

    /// `iprp` with the current `ipco`, every `ipma` merged into one.
    fn iprp(&self, iprp: &Child) -> Result<Vec<u8>> {
        let mut body = Vec::new();
        let mut ipma_done = false;
        for c in children(&self.meta, iprp.body()..iprp.range.end)? {
            match &c.kind {
                b"ipco" => body.extend(boxed(b"ipco", &self.props.concat())),
                b"ipma" if !ipma_done => {
                    body.extend(ipma_box(&self.ipma)?);
                    ipma_done = true;
                }
                b"ipma" => {}
                _ => body.extend_from_slice(&self.meta[c.range]),
            }
        }
        Ok(boxed(b"iprp", &body))
    }
}

/// Compose `irot`/`imir` properties, in association order, into an EXIF orientation.
fn compose<'a>(props: impl Iterator<Item = &'a [u8]>) -> Option<i16> {
    let (mut turns, mut flip, mut any) = (0u8, false, false);
    for p in props {
        let Some(&v) = p.get(8) else { continue };
        match &p[4..8] {
            b"irot" => turns = (turns + 4 - (v & 3)) % 4,
            b"imir" => {
                turns = (if v & 1 == 1 { 4 - turns } else { 6 - turns }) % 4;
                flip = !flip;
            }
            _ => continue,
        }
        any = true;
    }
    let i = DIHEDRAL.iter().position(|&d| d == (turns, flip))?;
    any.then_some(i as i16 + 1)
}

fn prop(props: &[Vec<u8>], index: u16) -> Option<&[u8]> {
    props
        .get(usize::from(index).checked_sub(1)?)
        .map(Vec::as_slice)
}

fn is_transform(p: Option<&[u8]>) -> bool {
    matches!(p.and_then(|p| p.get(4..8)), Some(b"irot" | b"imir"))
}

/// A one-byte property box.
fn property(kind: &[u8; 4], value: u8) -> Vec<u8> {
    boxed(kind, &[value])
}

/// The index of an identical property, appending it if there is none.
fn intern(props: &mut Vec<Vec<u8>>, p: Vec<u8>) -> u16 {
    let i = props.iter().position(|q| *q == p).unwrap_or_else(|| {
        props.push(p);
        props.len() - 1
    });
    i as u16 + 1
}

/// Drop `irot`/`imir` boxes nothing references any more, renumbering the associations.
fn drop_unused_transforms(props: &mut Vec<Vec<u8>>, ipma: &mut [Entry]) {
    let used: HashSet<u16> = ipma
        .iter()
        .flat_map(|e| &e.assocs)
        .map(|a| a.index)
        .collect();
    let mut remap = vec![0u16; props.len() + 1];
    let mut kept = Vec::with_capacity(props.len());
    for (i, p) in std::mem::take(props).into_iter().enumerate() {
        if is_transform(Some(p.as_slice())) && !used.contains(&(i as u16 + 1)) {
            continue;
        }
        kept.push(p);
        remap[i + 1] = kept.len() as u16;
    }
    *props = kept;
    for a in ipma.iter_mut().flat_map(|e| &mut e.assocs) {
        a.index = remap[usize::from(a.index)];
    }
}

fn parse_ipma(b: &[u8], c: &Child, out: &mut Vec<Entry>) -> Result<()> {
    let width = id_size(be(b, c.body(), 1)?);
    let wide = be(b, c.body() + 1, 3)? & 1 == 1;
    let (index_size, flag) = if wide { (2, 0x8000) } else { (1, 0x80) };
    let mut pos = c.body() + 4;
    let count = be(b, pos, 4)?;
    pos += 4;
    for _ in 0..count {
        let item = be(b, pos, width)? as u32;
        let n = be(b, pos + width, 1)?;
        pos += width + 1;
        let mut assocs = Vec::new();
        for _ in 0..n {
            let v = be(b, pos, index_size)?;
            pos += index_size;
            assocs.push(Assoc {
                index: (v & (flag - 1)) as u16,
                essential: v & flag != 0,
            });
        }
        out.push(Entry { item, assocs });
    }
    Ok(())
}

fn ipma_box(ipma: &[Entry]) -> Result<Vec<u8>> {
    let wide_ids = ipma.iter().any(|e| e.item > u32::from(u16::MAX));
    let wide = ipma.iter().flat_map(|e| &e.assocs).any(|a| a.index > 0x7F);
    let mut body = vec![u8::from(wide_ids), 0, 0, u8::from(wide)];
    body.extend((ipma.len() as u32).to_be_bytes());
    for e in ipma {
        if wide_ids {
            body.extend(e.item.to_be_bytes());
        } else {
            body.extend((e.item as u16).to_be_bytes());
        }
        body.push(u8::try_from(e.assocs.len()).map_err(|_| unsupported("over 255 properties"))?);
        for a in &e.assocs {
            if wide {
                body.extend((u16::from(a.essential) << 15 | a.index).to_be_bytes());
            } else {
                body.push(u8::from(a.essential) << 7 | a.index as u8);
            }
        }
    }
    Ok(boxed(b"ipma", &body))
}

// ── iloc ─────────────────────────────────────────────────────────────────────

/// An integer field of an `iloc` box.
#[derive(Clone, Copy)]
struct Field {
    at: usize,
    size: usize,
    value: u64,
}

/// One `iloc` item: `here` when its data is in this file at absolute offsets (construction method
/// 0, data reference 0), with its base offset and `(offset, length)` extents.
struct Location {
    here: bool,
    base: Field,
    extents: Vec<(Field, u64)>,
}

fn iloc_locations(b: &[u8], header: usize) -> Result<Vec<Location>> {
    let version = be(b, header, 1)?;
    let sizes = be(b, header + 4, 2)?;
    let nibble = |shift: u32| ((sizes >> shift) & 0xF) as usize;
    let (offset_size, length_size, base_size) = (nibble(12), nibble(8), nibble(4));
    let index_size = if version > 0 { nibble(0) } else { 0 };
    let width = if version < 2 { 2 } else { 4 };
    let mut pos = header + 6;
    let count = be(b, pos, width)?;
    pos += width;
    let field = |at: usize, size: usize| {
        Ok::<_, WorkerError>(Field {
            at,
            size,
            value: be(b, at, size)?,
        })
    };

    let mut out = Vec::new();
    for _ in 0..count {
        pos += width;
        let method = if version > 0 {
            pos += 2;
            be(b, pos - 2, 2)? & 0xF
        } else {
            0
        };
        let data_ref = be(b, pos, 2)?;
        let base = field(pos + 2, base_size)?;
        pos += 2 + base_size;
        let n = be(b, pos, 2)?;
        pos += 2;
        let mut extents = Vec::new();
        for _ in 0..n {
            pos += index_size;
            let offset = field(pos, offset_size)?;
            extents.push((offset, be(b, pos + offset_size, length_size)?));
            pos += offset_size + length_size;
        }
        out.push(Location {
            here: method == 0 && data_ref == 0,
            base,
            extents,
        });
    }
    Ok(out)
}

/// Move every location at or past the end of `meta` by `delta`: the base offset when it lies
/// there, else each extent.
fn shift_iloc(b: &mut [u8], header: usize, meta: &Range<u64>, delta: i64) -> Result<()> {
    for loc in iloc_locations(b, header)?.into_iter().filter(|l| l.here) {
        if loc.base.size > 0 && loc.base.value >= meta.end {
            put(b, loc.base, delta)?;
            continue;
        }
        for (offset, _) in loc.extents {
            let at = loc.base.value.saturating_add(offset.value);
            if at >= meta.end {
                put(b, offset, delta)?;
            } else if at >= meta.start && delta != 0 {
                return Err(unsupported("item data inside meta"));
            }
        }
    }
    Ok(())
}

fn put(b: &mut [u8], field: Field, delta: i64) -> Result<()> {
    let v = field
        .value
        .checked_add_signed(delta)
        .filter(|v| field.size >= 8 || v >> (8 * field.size) == 0)
        .ok_or_else(|| unsupported("an offset outgrowing its field"))?;
    for i in 0..field.size {
        b[field.at + i] = (v >> (8 * (field.size - 1 - i))) as u8;
    }
    Ok(())
}

// ── Boxes ────────────────────────────────────────────────────────────────────

/// A box inside a buffer; its payload starts `header` bytes in.
struct Child {
    kind: [u8; 4],
    range: Range<usize>,
    header: usize,
}

impl Child {
    fn body(&self) -> usize {
        self.range.start + self.header
    }
}

/// A box header: (type, header length, total size — 0 meaning "to the end of the parent").
fn header(b: &[u8]) -> Result<([u8; 4], usize, u64)> {
    let size = be(b, 0, 4)?;
    let kind = b
        .get(4..8)
        .and_then(|k| k.try_into().ok())
        .ok_or_else(|| malformed("truncated box"))?;
    let (head, size) = match size {
        1 => (16, be(b, 8, 8)?),
        n => (8, n),
    };
    if size != 0 && size < head as u64 {
        return Err(malformed("box smaller than its header"));
    }
    Ok((kind, head, size))
}

fn children(b: &[u8], range: Range<usize>) -> Result<Vec<Child>> {
    let mut out = Vec::new();
    let mut pos = range.start;
    while pos < range.end {
        let (kind, header, size) = header(&b[pos..range.end])?;
        let end = match size {
            0 => range.end,
            n => usize::try_from(n)
                .ok()
                .and_then(|n| pos.checked_add(n))
                .filter(|&end| end <= range.end)
                .ok_or_else(|| malformed("box overruns its parent"))?,
        };
        out.push(Child {
            kind,
            range: pos..end,
            header,
        });
        pos = end;
    }
    Ok(out)
}

/// The children of a `meta` box (a full box: version and flags precede them).
fn meta_children(meta: &[u8]) -> Result<Vec<Child>> {
    let (_, head, _) = header(meta)?;
    children(meta, head + 4..meta.len())
}

fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 16);
    match u32::try_from(body.len() + 8) {
        Ok(size) => {
            out.extend(size.to_be_bytes());
            out.extend(kind);
        }
        Err(_) => {
            out.extend(1u32.to_be_bytes());
            out.extend(kind);
            out.extend((body.len() as u64 + 16).to_be_bytes());
        }
    }
    out.extend_from_slice(body);
    out
}

/// Item IDs are 16-bit in version 0 of a full box, 32-bit after.
fn id_size(version: u64) -> usize {
    if version == 0 { 2 } else { 4 }
}

/// Big-endian unsigned integer of `n` bytes (0–8) at `pos`.
fn be(b: &[u8], pos: usize, n: usize) -> Result<u64> {
    let bytes = pos
        .checked_add(n)
        .and_then(|end| b.get(pos..end))
        .ok_or_else(|| malformed("truncated box"))?;
    Ok(bytes.iter().fold(0, |v, &x| v << 8 | u64::from(x)))
}

/// Replace `span` of the file with `meta`, copying the rest untouched.
fn splice(path: &Path, span: &Range<u64>, meta: &[u8]) -> Result<()> {
    let tmp = path.with_extension("heif-rewrite");
    let mut src = File::open(path)?;
    let mut dst = File::create(&tmp)?;
    io::copy(&mut (&mut src).take(span.start), &mut dst)?;
    dst.write_all(meta)?;
    src.seek(SeekFrom::Start(span.end))?;
    io::copy(&mut src, &mut dst)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn malformed(what: &str) -> WorkerError {
    WorkerError::UnsupportedFormat(format!("malformed HEIF: {what}"))
}

fn unsupported(what: &str) -> WorkerError {
    WorkerError::Exif(format!("HEIF orientation write refused: {what}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Encoded by libheif (`heif-enc`): no transform, a `clap`, a base-offset `iloc`.
    const PLAIN: &[u8] = include_bytes!("testdata/plain.heic");
    /// `heif-enc -t`: a thumbnail item referencing the primary.
    const THUMBNAIL: &[u8] = include_bytes!("testdata/thumbnail.heic");
    /// `plain.heic` re-oriented 1–8 by ImageIO (`CGImageDestinationCopyImageSource`).
    const IMAGEIO_FILES: [&[u8]; 8] = [
        include_bytes!("testdata/apple1.heic"),
        include_bytes!("testdata/apple2.heic"),
        include_bytes!("testdata/apple3.heic"),
        include_bytes!("testdata/apple4.heic"),
        include_bytes!("testdata/apple5.heic"),
        include_bytes!("testdata/apple6.heic"),
        include_bytes!("testdata/apple7.heic"),
        include_bytes!("testdata/apple8.heic"),
    ];

    fn file(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn open(path: &Path) -> Heif {
        Heif::open(path).unwrap().expect("a HEIF file")
    }

    /// An item's `irot`/`imir` boxes, in association order.
    fn transforms(heif: &Heif, item: u32) -> Vec<Vec<u8>> {
        let entry = heif.ipma.iter().find(|e| e.item == item).unwrap();
        entry
            .assocs
            .iter()
            .filter_map(|a| prop(&heif.props, a.index))
            .filter(|p| is_transform(Some(p)))
            .map(<[u8]>::to_vec)
            .collect()
    }

    /// The bytes every `iloc` extent points at: a rewrite must still find the same data.
    fn item_data(path: &Path) -> Vec<Vec<u8>> {
        let bytes = fs::read(path).unwrap();
        let heif = open(path);
        let iloc = meta_children(&heif.meta)
            .unwrap()
            .into_iter()
            .find(|c| &c.kind == b"iloc")
            .unwrap();
        let mut data = Vec::new();
        for loc in iloc_locations(&heif.meta[iloc.range], iloc.header).unwrap() {
            for (offset, len) in loc.extents.into_iter().filter(|_| loc.here) {
                let at = (loc.base.value + offset.value) as usize;
                data.push(bytes[at..at + len as usize].to_vec());
            }
        }
        data
    }

    #[test]
    fn imageio_encodings_compose_back() {
        for (i, &(angle, axis)) in IMAGEIO.iter().enumerate() {
            let mut props = vec![property(b"irot", angle)];
            props.extend(axis.map(|axis| property(b"imir", axis)));
            assert_eq!(compose(props.iter().map(Vec::as_slice)), Some(i as i16 + 1));
        }
    }

    #[test]
    fn reads_the_orientation_imageio_wrote() {
        let dir = tempfile::tempdir().unwrap();
        for (i, bytes) in IMAGEIO_FILES.iter().enumerate() {
            let path = file(&dir, &format!("{i}.heic"), bytes);
            assert_eq!(open(&path).orientation(), Some(i as i16 + 1));
        }
        assert_eq!(open(&file(&dir, "plain.heic", PLAIN)).orientation(), None);
    }

    #[test]
    fn writes_every_orientation_as_imageio_does() {
        let dir = tempfile::tempdir().unwrap();
        for (i, bytes) in IMAGEIO_FILES.iter().enumerate() {
            let o = i as i16 + 1;
            let path = file(&dir, "plain.heic", PLAIN);
            let data = item_data(&path);
            open(&path).write_orientation(&path, Some(o)).unwrap();

            let heif = open(&path);
            let reference = open(&file(&dir, "imageio.heic", bytes));
            assert_eq!(heif.orientation(), Some(o));
            assert_eq!(
                transforms(&heif, heif.primary),
                transforms(&reference, reference.primary),
                "orientation {o}"
            );
            assert_eq!(
                item_data(&path),
                data,
                "orientation {o} lost the media data"
            );
        }
    }

    #[test]
    fn thumbnails_turn_with_the_primary() {
        let dir = tempfile::tempdir().unwrap();
        let path = file(&dir, "thumbnail.heic", THUMBNAIL);
        let data = item_data(&path);
        open(&path).write_orientation(&path, Some(7)).unwrap();

        let heif = open(&path);
        assert_eq!(heif.targets.len(), 2);
        for &item in &heif.targets {
            let props = transforms(&heif, item);
            assert_eq!(
                compose(props.iter().map(Vec::as_slice)),
                Some(7),
                "item {item}"
            );
        }
        assert_eq!(item_data(&path), data);
    }

    #[test]
    fn rewrites_leave_no_residue() {
        let dir = tempfile::tempdir().unwrap();
        let path = file(&dir, "plain.heic", PLAIN);
        for o in [Some(5), Some(6), Some(2), None] {
            open(&path).write_orientation(&path, o).unwrap();
        }
        assert_eq!(fs::read(&path).unwrap(), PLAIN);

        let path = file(&dir, "mirrored.heic", IMAGEIO_FILES[4]);
        open(&path).write_orientation(&path, None).unwrap();
        let heif = open(&path);
        assert_eq!(heif.orientation(), None);
        assert!(!heif.props.iter().any(|p| is_transform(Some(p))));
    }

    #[test]
    fn other_files_are_not_heif() {
        let dir = tempfile::tempdir().unwrap();
        let mp4 = [&b"\0\0\0\x10ftypisom\0\0\0\0"[..], b"\0\0\0\x08mdat"].concat();
        for (name, bytes) in [
            ("a.jpg", &b"\xff\xd8\xff\xe0\0\x10"[..]),
            ("a.mp4", &mp4),
            ("empty", b""),
        ] {
            assert!(
                Heif::open(&file(&dir, name, bytes)).unwrap().is_none(),
                "{name}"
            );
        }
    }
}
