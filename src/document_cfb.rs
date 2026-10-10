//! Selective, evidence-bound CFB stream mapping. Missing directory entries are
//! skipped only outside the complete readable root sibling tree. No metadata,
//! sector link, stream name or byte is synthesized, and no CFB is rewritten.
use super::{SparseStream, u16at, u32at};
use crate::fat12;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const END: u32 = 0xffff_fffe;
const FREE: u32 = 0xffff_ffff;
const FAT: u32 = 0xffff_fffd;
const MAX_ENTRIES: usize = 4096;
const MAX_VISITS: usize = 65_536;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    pub allocation_table: String,
    pub sector_id: u32,
    pub next_sector_id: u32,
    pub source_byte_offset: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryEvidence {
    pub stream_id: u32,
    pub name: String,
    pub directory_parent_byte_offset: usize,
    pub raw_entry_hex: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEvidence {
    pub name: String,
    pub stream_id: u32,
    pub bytes: usize,
    pub mini_stream: bool,
    pub sector_ids: Vec<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub method: String,
    pub sector_bytes: usize,
    pub directory_sector_ids: Vec<u32>,
    pub missing_directory_stream_ids: Vec<u32>,
    pub root_sibling_entries: Vec<EntryEvidence>,
    pub allocation_links: Vec<Link>,
    pub selected_streams: Vec<StreamEvidence>,
    pub allocation_issues: Vec<String>,
    pub whole_container_verified: bool,
    pub warning: String,
}
#[derive(Clone)]
struct Entry {
    id: u32,
    name: String,
    kind: u8,
    left: u32,
    right: u32,
    child: u32,
    start: u32,
    length: usize,
    offset: usize,
    raw: Vec<u8>,
}
struct Walk {
    ids: Vec<u32>,
    error: Option<String>,
}
impl Walk {
    fn complete(self) -> Result<Vec<u32>, String> {
        match self.error {
            Some(e) => Err(e),
            None => Ok(self.ids),
        }
    }
}
struct Parser<'a> {
    bytes: &'a [u8],
    sources: &'a [Option<usize>],
    sector: usize,
    fats: Vec<u32>,
    metadata: BTreeSet<u64>,
    links: BTreeMap<(bool, u32), Link>,
    visits: usize,
}
impl Parser<'_> {
    fn known(&mut self, at: usize, n: usize) -> Result<Vec<u8>, String> {
        let end = at.checked_add(n).ok_or("CFB metadata range overflow")?;
        let bytes = self
            .bytes
            .get(at..end)
            .ok_or("CFB metadata outside parent")?;
        if self.sources[at..end].iter().any(Option::is_none) {
            return Err("Required CFB metadata touches missing/unmapped bytes".into());
        }
        self.metadata.extend(
            self.sources[at..end]
                .iter()
                .flatten()
                .map(|b| (*b / 512) as u64),
        );
        Ok(bytes.to_vec())
    }
    fn offset(&self, sid: u32) -> Result<usize, String> {
        let at = (sid as usize + 1)
            .checked_mul(self.sector)
            .ok_or("CFB sector overflow")?;
        if at >= self.bytes.len() || self.fats.contains(&sid) {
            return Err("CFB sector is outside parent or reserved as FAT".into());
        }
        Ok(at)
    }
    fn next(&mut self, sid: u32, mini: Option<&SparseStream>) -> Result<u32, String> {
        let (bytes, source) = if let Some(table) = mini {
            let at = sid as usize * 4;
            let b = table
                .bytes
                .get(at..at + 4)
                .ok_or("Mini FAT entry outside table")?;
            let source = table
                .sources
                .get(at..at + 4)
                .ok_or("Mini FAT mapping outside table")?;
            if source.iter().any(Option::is_none) {
                return Err("Mini FAT link is unreadable; not guessed".into());
            }
            self.metadata
                .extend(source.iter().flatten().map(|b| (*b / 512) as u64));
            (b.to_vec(), source[0].unwrap())
        } else {
            let per = self.sector / 4;
            let fat = *self
                .fats
                .get(sid as usize / per)
                .ok_or("CFB FAT entry outside DIFAT coverage")?;
            let at = (fat as usize + 1) * self.sector + sid as usize % per * 4;
            let b = self.known(at, 4)?;
            (b, self.sources[at].unwrap())
        };
        let next = u32at(&bytes, 0) as u32;
        self.links
            .entry((mini.is_some(), sid))
            .or_insert_with(|| Link {
                allocation_table: if mini.is_some() { "mini_fat" } else { "fat" }.into(),
                sector_id: sid,
                next_sector_id: next,
                source_byte_offset: source,
            });
        Ok(next)
    }
    fn walk(
        &mut self,
        start: u32,
        count: Option<usize>,
        mini: Option<&SparseStream>,
        mini_bytes: usize,
    ) -> Walk {
        let mut ids = Vec::new();
        let mut seen = BTreeSet::new();
        let mut sid = start;
        let result = (|| -> Result<(), String> {
            while sid != END {
                self.visits += 1;
                if self.visits > MAX_VISITS {
                    return Err("CFB allocation visit ceiling".into());
                }
                if !seen.insert(sid) {
                    return Err("CFB allocation cycle".into());
                }
                if mini.is_some() {
                    if sid as usize >= mini_bytes.div_ceil(64) {
                        return Err("Mini sector outside root mini stream".into());
                    }
                } else {
                    self.offset(sid)?;
                }
                ids.push(sid);
                if ids.len() > count.unwrap_or(MAX_ENTRIES * 128 / self.sector) {
                    return Err("CFB allocation exceeds declared length/search ceiling".into());
                }
                sid = self.next(sid, mini)?;
            }
            if count.is_some_and(|n| ids.len() != n) {
                return Err("CFB allocation shorter than declared length".into());
            }
            Ok(())
        })();
        Walk {
            ids,
            error: result.err(),
        }
    }
    fn regular(&self, ids: &[u32], length: usize) -> Result<SparseStream, String> {
        let mut result = SparseStream {
            bytes: Vec::with_capacity(length),
            sources: Vec::with_capacity(length),
        };
        for sid in ids {
            let at = self.offset(*sid)?;
            let n = self.sector.min(length.saturating_sub(result.bytes.len()));
            let data = self
                .bytes
                .get(at..at + n)
                .ok_or("CFB stream outside parent")?;
            result.bytes.extend_from_slice(data);
            result.sources.extend_from_slice(&self.sources[at..at + n]);
        }
        if result.bytes.len() != length {
            return Err("CFB stream length mismatch".into());
        }
        Ok(result)
    }
}

fn parse_entry(raw: Vec<u8>, id: u32, offset: usize, major: u16) -> Result<Option<Entry>, String> {
    if raw[66] == 0 {
        return Ok(None);
    }
    let n = u16at(&raw, 64) as usize;
    if !(2..=64).contains(&n)
        || !n.is_multiple_of(2)
        || raw[n - 2..n] != [0, 0]
        || ![1, 2, 5].contains(&raw[66])
        || raw[67] > 1
    {
        return Err("Invalid readable CFB directory entry".into());
    }
    let units = raw[..n - 2]
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes(b.try_into().unwrap()))
        .collect::<Vec<_>>();
    let name = String::from_utf16(&units).map_err(|_| "Invalid CFB directory name UTF-16")?;
    if name.is_empty() || name.contains(['\0', '/', '\\', ':', '!']) {
        return Err("Invalid CFB directory name".into());
    }
    let length = if major == 3 {
        u32at(&raw, 120) as u64
    } else {
        u64::from_le_bytes(raw[120..128].try_into().unwrap())
    };
    if length > fat12::MAX_IMAGE_BYTES as u64 {
        return Err("CFB stream length ceiling".into());
    }
    Ok(Some(Entry {
        id,
        name,
        kind: raw[66],
        left: u32at(&raw, 68) as u32,
        right: u32at(&raw, 72) as u32,
        child: u32at(&raw, 76) as u32,
        start: u32at(&raw, 116) as u32,
        length: length as usize,
        offset,
        raw,
    }))
}

pub(super) fn streams(
    bytes: &[u8],
    sources: &[Option<usize>],
) -> Result<(SparseStream, SparseStream, BTreeSet<u64>, Evidence), String> {
    if bytes.len() > fat12::MAX_IMAGE_BYTES
        || sources.len() != bytes.len()
        || bytes.len() < 512
        || sources[..512].iter().any(Option::is_none)
    {
        return Err("Selective CFB requires a bounded readable header".into());
    }
    let header = &bytes[..512];
    let major = u16at(header, 26);
    let sector = match (major, u16at(header, 30)) {
        (3, 9) => 512,
        (4, 12) => 4096,
        _ => return Err("Unsupported CFB version/sector size".into()),
    };
    if header[..8] != [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]
        || header[8..24].iter().any(|b| *b != 0)
        || u16at(header, 28) != 0xfffe
        || u16at(header, 32) != 6
        || header[34..40].iter().any(|b| *b != 0)
        || u32at(header, 56) != 4096
        || u32at(header, 72) != 0
        || u32at(header, 68) as u32 != END
        || !bytes.len().is_multiple_of(sector)
        || (major == 3 && u32at(header, 40) != 0)
    {
        return Err("Inconsistent CFB header or unsupported external DIFAT".into());
    }
    let nfat = u32at(header, 44);
    if nfat == 0 || nfat > 109 || nfat * sector / 4 < bytes.len() / sector - 1 {
        return Err("CFB DIFAT count/coverage invalid".into());
    }
    let fats = (0..nfat)
        .map(|i| u32at(header, 76 + i * 4) as u32)
        .collect::<Vec<_>>();
    if fats.iter().copied().collect::<BTreeSet<_>>().len() != fats.len()
        || fats
            .iter()
            .any(|sid| *sid as usize >= bytes.len() / sector - 1)
        || (nfat..109).any(|i| u32at(header, 76 + i * 4) as u32 != FREE)
    {
        return Err("CFB DIFAT identities invalid/duplicated".into());
    }
    let mut p = Parser {
        bytes,
        sources,
        sector,
        fats,
        metadata: BTreeSet::new(),
        links: BTreeMap::new(),
        visits: 0,
    };
    p.known(0, sector)?; // v4 header padding must also be readable/zero
    if bytes[512..sector].iter().any(|b| *b != 0) {
        return Err("CFB header padding invalid".into());
    }
    for sid in p.fats.clone() {
        if p.next(sid, None)? != FAT {
            return Err("CFB FAT sector lacks allocation marker".into());
        }
    }
    let directory = p.walk(u32at(header, 48) as u32, None, None, 0).complete()?;
    if directory.len() * sector / 128 > MAX_ENTRIES
        || (major == 4 && directory.len() != u32at(header, 40))
    {
        return Err("CFB directory count/ceiling invalid".into());
    }
    let mut entries = Vec::new();
    let mut missing = Vec::new();
    for sid in &directory {
        let begin = p.offset(*sid)?;
        for off in (0..sector).step_by(128) {
            let id = entries.len() as u32;
            let at = begin + off;
            if sources[at..at + 128].iter().any(Option::is_none) {
                missing.push(id);
                entries.push(None);
            } else {
                entries.push(parse_entry(p.known(at, 128)?, id, at, major)?);
            }
        }
    }
    let root = entries
        .first()
        .and_then(Option::as_ref)
        .ok_or("CFB root entry unavailable")?
        .clone();
    if root.kind != 5 || root.name != "Root Entry" || root.left != FREE || root.right != FREE {
        return Err("CFB root entry invalid".into());
    }
    let mut root_ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut queue = vec![root.child];
    while let Some(id) = queue.pop() {
        if id == FREE {
            continue;
        }
        if id == 0 || !root_ids.insert(id) {
            return Err("CFB root sibling cycle/shared node".into());
        }
        let e = entries.get(id as usize).and_then(Option::as_ref).ok_or(
            "CFB root sibling path touches missing/invalid entry; no guessed stream scope",
        )?;
        if e.kind == 5 || !names.insert(e.name.to_uppercase()) || (e.kind == 2 && e.child != FREE) {
            return Err("CFB root stream identity ambiguous/invalid".into());
        }
        queue.extend([e.left, e.right]);
    }
    let find = |name: &str| -> Result<Entry, String> {
        root_ids
            .iter()
            .filter_map(|id| entries[*id as usize].as_ref())
            .find(|e| e.name.eq_ignore_ascii_case(name) && e.kind == 2)
            .cloned()
            .ok_or_else(|| format!("CFB readable root has no {name} stream"))
    };
    let word_entry = find("WordDocument")?;
    let nmini = u32at(header, 64);
    if nmini > bytes.len() / sector {
        return Err("CFB mini FAT count ceiling".into());
    }
    let mini_fat_ids = if nmini == 0 {
        if u32at(header, 60) as u32 != END {
            return Err("CFB empty mini FAT identity invalid".into());
        }
        vec![]
    } else {
        p.walk(u32at(header, 60) as u32, Some(nmini), None, 0)
            .complete()?
    };
    let mini_fat = p.regular(&mini_fat_ids, nmini * sector)?;
    let root_ids_all = if root.length == 0 {
        vec![]
    } else {
        p.walk(root.start, Some(root.length.div_ceil(sector)), None, 0)
            .complete()?
    };
    let root_mini = p.regular(&root_ids_all, root.length)?;
    let mut reserved = directory.iter().copied().collect::<BTreeSet<_>>();
    for ids in [&mini_fat_ids, &root_ids_all] {
        for id in ids {
            if !reserved.insert(*id) {
                return Err("CFB metadata/mini-stream allocation overlap".into());
            }
        }
    }
    let mut regular_claims = BTreeMap::<u32, BTreeSet<u32>>::new();
    let mut mini_claims = BTreeMap::<u32, BTreeSet<u32>>::new();
    let mut walks = BTreeMap::new();
    let mut issues = Vec::new();
    for entry in entries
        .iter()
        .flatten()
        .filter(|e| e.kind == 2 && e.length > 0)
    {
        let mini = entry.length < 4096;
        let walk = p.walk(
            entry.start,
            Some(entry.length.div_ceil(if mini { 64 } else { sector })),
            if mini { Some(&mini_fat) } else { None },
            root.length,
        );
        let claims = if mini {
            &mut mini_claims
        } else {
            &mut regular_claims
        };
        for id in &walk.ids {
            claims.entry(*id).or_default().insert(entry.id);
        }
        if let Some(error) = &walk.error {
            issues.push(format!("stream {} ({}): {error}", entry.id, entry.name));
        }
        walks.insert(entry.id, walk);
    }
    if p.visits > MAX_VISITS {
        return Err("CFB global allocation visit ceiling".into());
    }
    if reserved.iter().any(|sid| regular_claims.contains_key(sid)) {
        return Err("CFB stream crosslinks directory/mini-stream metadata allocation".into());
    }
    let mut selected = Vec::new();
    let mut read = |entry: &Entry| -> Result<SparseStream, String> {
        if entry.length == 0 {
            return Err("CFB selected stream empty".into());
        }
        let ids = walks
            .remove(&entry.id)
            .ok_or("CFB selected stream unmapped")?
            .complete()?;
        let mini = entry.length < 4096;
        let claims = if mini { &mini_claims } else { &regular_claims };
        if ids.iter().any(|id| {
            claims.get(id).is_some_and(|owners| owners.len() > 1)
                || (!mini && reserved.contains(id))
        }) {
            return Err("CFB selected stream crosslinks known allocation/metadata".into());
        }
        selected.push(StreamEvidence {
            name: entry.name.clone(),
            stream_id: entry.id,
            bytes: entry.length,
            mini_stream: mini,
            sector_ids: ids.clone(),
        });
        if !mini {
            return p.regular(&ids, entry.length);
        }
        let mut result = SparseStream {
            bytes: vec![],
            sources: vec![],
        };
        for id in &ids {
            let at = *id as usize * 64;
            let n = 64.min(entry.length - result.bytes.len());
            result.bytes.extend_from_slice(
                root_mini
                    .bytes
                    .get(at..at + n)
                    .ok_or("Mini stream outside root extent")?,
            );
            result
                .sources
                .extend_from_slice(&root_mini.sources[at..at + n]);
        }
        Ok(result)
    };
    let word = read(&word_entry)?;
    let fib = super::known(&word, 0, 426)?;
    let table_entry = find(if u16at(fib, 10) & 0x200 != 0 {
        "1Table"
    } else {
        "0Table"
    })?;
    let table = read(&table_entry)?;
    let mut root_evidence = vec![root];
    root_evidence.extend(
        root_ids
            .iter()
            .filter_map(|id| entries[*id as usize].clone()),
    );
    let evidence = Evidence { method:"selective_readable_cfb_root_streams".into(),sector_bytes:sector,
        directory_sector_ids:directory,missing_directory_stream_ids:missing,
        root_sibling_entries:root_evidence.into_iter().map(|e| EntryEvidence {stream_id:e.id,name:e.name,directory_parent_byte_offset:e.offset,raw_entry_hex:e.raw.iter().map(|b|format!("{b:02x}")).collect()}).collect(),
        allocation_links:p.links.into_values().collect(),selected_streams:selected,allocation_issues:issues,
        whole_container_verified:false,
        warning:"Selective stream salvage, NOT compound-container repair. Readable complete root sibling paths identify Word/table streams; missing non-root entries and unrelated allocation failures remain unresolved. Known allocation crosslinks are refused, but unavailable entries cannot establish every historical ownership claim. Only recorded links/extents are followed; no missing entry/link/name is synthesized, no macros/embedded objects are executed and no original semantics certified.".into() };
    Ok((word, table, p.metadata, evidence))
}
