//! Conservative, image-only FAT12 traversal. Unknown bytes are never file data.
//! Layout/entry reference: https://threadx.io/releases/6.5.1/filex/main/chapter3.html
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const SECTOR: usize = 512;
const MAX_ENTRIES: usize = 16_384;
pub const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layout {
    pub total_sectors: usize,
    pub sectors_per_cluster: usize,
    pub reserved_sectors: usize,
    pub fat_copies: usize,
    pub sectors_per_fat: usize,
    pub root_entries: usize,
    pub root_start: usize,
    pub data_start: usize,
    pub clusters: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FatLink {
    pub cluster: u16,
    pub next: u16,
    pub copies: Vec<usize>,
    pub source_lbas: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub path: String,
    pub short_name_hex: String,
    pub name_method: String,
    pub bytes: usize,
    pub sha256: String,
    pub directory_entry_offset: usize,
    pub metadata_lbas: Vec<u64>,
    pub data_lbas: Vec<u64>,
    pub clusters: Vec<u16>,
    pub fat_links: Vec<FatLink>,
    pub modified_dos_date: u16,
    pub modified_dos_time: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Analysis {
    pub layout: Layout,
    pub bad_lbas: Vec<u64>,
    pub directory_gaps: Vec<u64>,
    pub recovered_files: Vec<FileRecord>,
    pub skipped: Vec<Issue>,
    pub deleted_entries_not_recovered: usize,
    pub long_name_entries_not_used: usize,
    pub crosslinked_clusters: Vec<u16>,
    pub customer_delivery_certified: bool,
}

impl Layout {
    fn parse(image: &[u8], bad: &BTreeSet<u64>) -> Result<Self, String> {
        if image.len() < SECTOR
            || image.len() > MAX_IMAGE_BYTES
            || !image.len().is_multiple_of(SECTOR)
        {
            return Err(
                "Native FAT12 requires a complete 512-byte-sector image no larger than 4 MiB"
                    .into(),
            );
        }
        if bad.contains(&0) {
            return Err("Boot sector is unreadable; layout will not be guessed".into());
        }
        if word(image, 11) != 512 || image[510..512] != [0x55, 0xaa] || dword(image, 28) != 0 {
            return Err("Unsupported/invalid FAT12 boot sector (512-byte sectors, no partition offset required)".into());
        }
        let spc = image[13] as usize;
        let reserved = word(image, 14) as usize;
        let fats = image[16] as usize;
        let entries = word(image, 17) as usize;
        let spf = word(image, 22) as usize;
        let total = if word(image, 19) != 0 {
            word(image, 19) as usize
        } else {
            dword(image, 32) as usize
        };
        if !spc.is_power_of_two()
            || spc > 64
            || reserved == 0
            || reserved > 32
            || !(1..=2).contains(&fats)
            || entries == 0
            || entries > 4096
            || spf == 0
            || total != image.len() / SECTOR
        {
            return Err("Invalid BPB or image length differs from declared FAT12 volume".into());
        }
        let root_start = reserved + fats * spf;
        let data_start = root_start + (entries * 32).div_ceil(SECTOR);
        if data_start >= total {
            return Err("FAT/root directory extends past volume".into());
        }
        let clusters = (total - data_start) / spc;
        if clusters == 0 || clusters >= 4085 || ((clusters + 2) * 3).div_ceil(2) > spf * SECTOR {
            return Err("Volume is not a bounded FAT12 layout or FAT is too short".into());
        }
        Ok(Self {
            total_sectors: total,
            sectors_per_cluster: spc,
            reserved_sectors: reserved,
            fat_copies: fats,
            sectors_per_fat: spf,
            root_entries: entries,
            root_start,
            data_start,
            clusters,
        })
    }

    fn valid_cluster(&self, c: u16) -> bool {
        c >= 2 && (c as usize) < self.clusters + 2 && c < 0xff0
    }
    fn cluster_start(&self, c: u16) -> usize {
        self.data_start + (c as usize - 2) * self.sectors_per_cluster
    }

    fn link(&self, image: &[u8], bad: &BTreeSet<u64>, cluster: u16) -> Result<FatLink, String> {
        let offset = cluster as usize * 3 / 2;
        let mut values = Vec::new();
        for copy in 0..self.fat_copies {
            let start = (self.reserved_sectors + copy * self.sectors_per_fat) * SECTOR;
            let at = start + offset;
            let lbas = [(at / SECTOR) as u64, ((at + 1) / SECTOR) as u64]
                .into_iter()
                .collect::<BTreeSet<_>>();
            if lbas.iter().any(|lba| bad.contains(lba)) {
                continue;
            }
            let packed = word(image, at);
            let next = if cluster.is_multiple_of(2) {
                packed & 0xfff
            } else {
                packed >> 4
            };
            values.push((copy, next, lbas));
        }
        let Some(first) = values.first() else {
            return Err(format!(
                "FAT entry for cluster {cluster} is unreadable in every copy"
            ));
        };
        if values.iter().any(|(_, next, _)| *next != first.1) {
            return Err(format!(
                "Readable FAT copies disagree at cluster {cluster}; no majority/guess used"
            ));
        }
        Ok(FatLink {
            cluster,
            next: first.1,
            copies: values.iter().map(|v| v.0).collect(),
            source_lbas: values
                .iter()
                .flat_map(|v| v.2.iter().copied())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        })
    }

    fn chain(&self, image: &[u8], bad: &BTreeSet<u64>, start: u16) -> Chain {
        let mut result = Chain {
            clusters: Vec::new(),
            links: Vec::new(),
            error: None,
        };
        let mut seen = BTreeSet::new();
        let mut current = start;
        loop {
            if !self.valid_cluster(current) || !seen.insert(current) {
                result.error = Some(format!(
                    "Invalid/reserved cluster or chain cycle at {current}"
                ));
                break;
            }
            result.clusters.push(current);
            match self.link(image, bad, current) {
                Ok(link) => {
                    current = link.next;
                    result.links.push(link);
                    if current >= 0xff8 {
                        break;
                    }
                }
                Err(error) => {
                    result.error = Some(error);
                    break;
                }
            }
        }
        result
    }
}

struct Chain {
    clusters: Vec<u16>,
    links: Vec<FatLink>,
    error: Option<String>,
}
struct Directory {
    path: String,
    offsets: Vec<usize>,
    ancestors: Vec<u16>,
    metadata: BTreeSet<u64>,
    depth: usize,
}
struct Candidate {
    record: FileRecord,
    ancestors: Vec<u16>,
    issue: Option<String>,
}

/// Return only complete file records. Callers may materialize bytes with file_bytes.
pub fn analyze(image: &[u8], bad_lbas: &[u64]) -> Result<Analysis, String> {
    let bad = bad_lbas.iter().copied().collect::<BTreeSet<_>>();
    if bad.len() != bad_lbas.len() || bad.iter().any(|lba| *lba >= (image.len() / SECTOR) as u64) {
        return Err("Invalid, duplicate or out-of-range bad-sector evidence".into());
    }
    let layout = Layout::parse(image, &bad)?;
    let mut result = Analysis {
        layout: layout.clone(),
        bad_lbas: bad.into_iter().collect(),
        directory_gaps: Vec::new(),
        recovered_files: Vec::new(),
        skipped: Vec::new(),
        deleted_entries_not_recovered: 0,
        long_name_entries_not_used: 0,
        crosslinked_clusters: Vec::new(),
        customer_delivery_certified: false,
    };
    let bad = result.bad_lbas.iter().copied().collect::<BTreeSet<_>>();
    let mut queue = VecDeque::from([Directory {
        path: String::new(),
        offsets: (0..layout.root_entries)
            .map(|i| layout.root_start * SECTOR + i * 32)
            .collect(),
        ancestors: Vec::new(),
        metadata: BTreeSet::from([0]),
        depth: 0,
    }]);
    let mut directory_seen = BTreeSet::new();
    let mut owners: BTreeMap<u16, BTreeSet<String>> = BTreeMap::new();
    let mut candidates = Vec::new();
    let mut gaps = BTreeSet::new();
    let mut entries_seen = 0;
    let mut chain_visits = 0usize;
    let mut paths = BTreeMap::<String, usize>::new();
    while let Some(dir) = queue.pop_front() {
        for at in dir.offsets {
            entries_seen += 1;
            if entries_seen > MAX_ENTRIES {
                return Err(
                    "Directory traversal entry ceiling reached; no extraction published".into(),
                );
            }
            let lba = (at / SECTOR) as u64;
            if bad.contains(&lba) {
                gaps.insert(lba);
                continue;
            }
            let entry = &image[at..at + 32];
            if entry[0] == 0 {
                break;
            } // Only an intact end marker ends this directory.
            if entry[0] == 0xe5 {
                result.deleted_entries_not_recovered += 1;
                continue;
            }
            if entry[11] == 0x0f {
                result.long_name_entries_not_used += 1;
                continue;
            }
            if entry[11] & 8 != 0 {
                continue;
            } // volume label
            if entry[..11] == *b".          " || entry[..11] == *b"..         " {
                continue;
            }
            let short = short_name(&entry[..11]);
            let path = if dir.path.is_empty() {
                short
            } else {
                format!("{}/{short}", dir.path)
            };
            *paths.entry(path.to_lowercase()).or_default() += 1;
            let mut metadata = dir.metadata.clone();
            metadata.insert(lba);
            if entry[11] & 0xc0 != 0 || word(entry, 20) != 0 {
                result.skipped.push(Issue {
                    path,
                    reason: "Unsupported directory attributes/high cluster word".into(),
                });
                continue;
            }
            let start = word(entry, 26);
            let is_dir = entry[11] & 0x10 != 0;
            let size = dword(entry, 28) as usize;
            let chain = if !is_dir && size == 0 && start == 0 {
                Chain {
                    clusters: Vec::new(),
                    links: Vec::new(),
                    error: None,
                }
            } else {
                layout.chain(image, &bad, start)
            };
            chain_visits += chain.clusters.len();
            if chain_visits > 65_536 {
                return Err("Allocation traversal ceiling reached; no extraction published".into());
            }
            for c in &chain.clusters {
                owners
                    .entry(*c)
                    .or_default()
                    .insert(format!("{}:{at}", path));
            }
            metadata.extend(
                chain
                    .links
                    .iter()
                    .flat_map(|l| l.source_lbas.iter().copied()),
            );
            if is_dir {
                if dir.depth >= 32 || !directory_seen.insert(start) {
                    result.skipped.push(Issue {
                        path,
                        reason: "Directory recursion/duplicate cluster refused".into(),
                    });
                    continue;
                }
                if let Some(error) = &chain.error {
                    result.skipped.push(Issue {
                        path: path.clone(),
                        reason: format!("Directory tail unavailable: {error}"),
                    });
                }
                let mut ancestors = dir.ancestors.clone();
                ancestors.extend(&chain.clusters);
                let offsets = chain
                    .clusters
                    .iter()
                    .flat_map(|c| {
                        let begin = layout.cluster_start(*c) * SECTOR;
                        (0..layout.sectors_per_cluster * SECTOR / 32).map(move |i| begin + i * 32)
                    })
                    .collect();
                queue.push_back(Directory {
                    path,
                    offsets,
                    ancestors,
                    metadata,
                    depth: dir.depth + 1,
                });
                continue;
            }
            let data_lbas = chain
                .clusters
                .iter()
                .flat_map(|c| {
                    let begin = layout.cluster_start(*c);
                    begin..begin + layout.sectors_per_cluster
                })
                .take(size.div_ceil(SECTOR))
                .map(|lba| lba as u64)
                .collect::<Vec<_>>();
            let needed = size.div_ceil(layout.sectors_per_cluster * SECTOR);
            let issue = chain.error.or_else(|| {
                if size > image.len() || chain.clusters.len() != needed {
                    Some("File size and terminated allocation chain disagree".into())
                } else if data_lbas.iter().any(|lba| bad.contains(lba)) {
                    Some(
                        "File content intersects unreadable/conflicting acquisition sectors".into(),
                    )
                } else {
                    None
                }
            });
            candidates.push(Candidate {
                record: FileRecord {
                    path,
                    short_name_hex: hex(&entry[..11]),
                    name_method: "8.3; non-ASCII OEM bytes escaped, long names not reconstructed"
                        .into(),
                    bytes: size,
                    sha256: String::new(),
                    directory_entry_offset: at,
                    metadata_lbas: metadata.into_iter().collect(),
                    data_lbas,
                    clusters: chain.clusters,
                    fat_links: chain.links,
                    modified_dos_date: word(entry, 24),
                    modified_dos_time: word(entry, 22),
                },
                ancestors: dir.ancestors.clone(),
                issue,
            });
        }
    }
    let crosslinks = owners
        .into_iter()
        .filter(|(_, claims)| claims.len() > 1)
        .map(|(c, _)| c)
        .collect::<BTreeSet<_>>();
    result.crosslinked_clusters = crosslinks.iter().copied().collect();
    result.directory_gaps = gaps.into_iter().collect();
    for mut candidate in candidates {
        let r = &candidate.record;
        let reason = candidate.issue.or_else(|| {
            if r.clusters
                .iter()
                .chain(&candidate.ancestors)
                .any(|c| crosslinks.contains(c))
            {
                Some("Cross-linked file/directory allocation; ownership ambiguous".into())
            } else if paths[&r.path.to_lowercase()] > 1 {
                Some("Case-insensitive duplicate path; neither entry selected".into())
            } else if ambiguous_parent(&r.path, &paths) {
                Some("Directory path has duplicate owners".into())
            } else {
                None
            }
        });
        if let Some(reason) = reason {
            result.skipped.push(Issue {
                path: r.path.clone(),
                reason,
            });
        } else {
            candidate.record.sha256 =
                format!("{:x}", Sha256::digest(file_bytes(image, &candidate.record)));
            result.recovered_files.push(candidate.record);
        }
    }
    result.recovered_files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}

fn ambiguous_parent(path: &str, paths: &BTreeMap<String, usize>) -> bool {
    let mut parent = path;
    while let Some((prefix, _)) = parent.rsplit_once('/') {
        if paths
            .get(&prefix.to_lowercase())
            .is_some_and(|count| *count > 1)
        {
            return true;
        }
        parent = prefix;
    }
    false
}

pub(crate) fn file_bytes(image: &[u8], record: &FileRecord) -> Vec<u8> {
    record
        .data_lbas
        .iter()
        .flat_map(|lba| {
            image[*lba as usize * SECTOR..(*lba as usize + 1) * SECTOR]
                .iter()
                .copied()
        })
        .take(record.bytes)
        .collect()
}

/// Each name stays one safe path component. Escape, don't guess OEM encoding.
fn short_name(raw: &[u8]) -> String {
    let part = |bytes: &[u8]| {
        let end = bytes.iter().rposition(|b| *b != b' ').map_or(0, |i| i + 1);
        let mut name = String::new();
        for (i, b) in bytes[..end].iter().enumerate() {
            // Preserve ordinary underscores (notably DOS installer .EX_/.DL_ names).
            // Escape literal escape prefixes so raw _xHH cannot alias an escaped byte.
            let escape_prefix = *b == b'_'
                && i + 3 < end
                && bytes[i + 1].eq_ignore_ascii_case(&b'x')
                && bytes[i + 2].is_ascii_hexdigit()
                && bytes[i + 3].is_ascii_hexdigit();
            if !escape_prefix && (b.is_ascii_alphanumeric() || b"_!#$%&'()-@^`{}~".contains(b)) {
                name.push(*b as char);
            } else {
                name.push_str(&format!("_x{b:02X}"));
            }
        }
        name
    };
    let base = part(&raw[..8]);
    let ext = part(&raw[8..]);
    let name = if ext.is_empty() {
        base
    } else {
        format!("{base}.{ext}")
    };
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if name.is_empty()
        || name.starts_with('.')
        || [
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ]
        .contains(&stem.as_str())
    {
        format!("_FAT_{}", hex(raw))
    } else {
        name
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}
fn word(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}
fn dword(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub fn image() -> Vec<u8> {
        let mut image = vec![0; 2880 * SECTOR];
        image[0..3].copy_from_slice(&[0xeb, 0x3c, 0x90]);
        image[11..13].copy_from_slice(&512u16.to_le_bytes());
        image[13] = 1;
        image[14..16].copy_from_slice(&1u16.to_le_bytes());
        image[16] = 2;
        image[17..19].copy_from_slice(&224u16.to_le_bytes());
        image[19..21].copy_from_slice(&2880u16.to_le_bytes());
        image[21] = 0xf0;
        image[22..24].copy_from_slice(&9u16.to_le_bytes());
        image[510..512].copy_from_slice(&[0x55, 0xaa]);
        for sector in [1, 10] {
            image[sector * 512..sector * 512 + 3].copy_from_slice(&[0xf0, 0xff, 0xff]);
        }
        image
    }
    pub fn set_fat(image: &mut [u8], copy: usize, cluster: u16, next: u16) {
        let at = (1 + copy * 9) * 512 + cluster as usize * 3 / 2;
        let packed = word(image, at);
        let packed = if cluster.is_multiple_of(2) {
            (packed & 0xf000) | next
        } else {
            (packed & 0x000f) | (next << 4)
        };
        image[at..at + 2].copy_from_slice(&packed.to_le_bytes());
    }
    pub fn entry(image: &mut [u8], at: usize, name: &[u8; 11], cluster: u16, size: u32, dir: bool) {
        image[at..at + 11].copy_from_slice(name);
        image[at + 11] = if dir { 0x10 } else { 0x20 };
        image[at + 26..at + 28].copy_from_slice(&cluster.to_le_bytes());
        image[at + 28..at + 32].copy_from_slice(&size.to_le_bytes());
    }
    pub fn file(image: &mut [u8], at: usize, name: &[u8; 11], cluster: u16, bytes: &[u8]) {
        entry(image, at, name, cluster, bytes.len() as u32, false);
        for copy in 0..2 {
            set_fat(image, copy, cluster, 0xfff);
        }
        let begin = (33 + cluster as usize - 2) * 512;
        image[begin..begin + bytes.len()].copy_from_slice(bytes);
    }
    #[test]
    fn fragmented_file_exact_bytes_and_sector_provenance() {
        let mut img = image();
        entry(&mut img, 19 * 512, b"FRAGMENTTXT", 2, 600, false);
        for copy in 0..2 {
            set_fat(&mut img, copy, 2, 5);
            set_fat(&mut img, copy, 5, 0xfff);
        }
        img[33 * 512..34 * 512].fill(0x11);
        img[36 * 512..37 * 512].fill(0x22);
        let found = analyze(&img, &[]).unwrap();
        let r = &found.recovered_files[0];
        assert_eq!(r.path, "FRAGMENT.TXT");
        assert_eq!(r.clusters, vec![2, 5]);
        assert_eq!(r.data_lbas, vec![33, 36]);
        assert_eq!(r.fat_links[0].copies, vec![0, 1]);
        let bytes = file_bytes(&img, r);
        assert_eq!(bytes.len(), 600);
        assert!(bytes[..512].iter().all(|b| *b == 0x11));
        assert!(bytes[512..].iter().all(|b| *b == 0x22));
    }
    #[test]
    fn unreadable_directory_sector_is_skipped_not_end_marker() {
        let mut img = image();
        file(&mut img, 20 * 512, b"AFTER   TXT", 2, b"intact");
        let found = analyze(&img, &[19]).unwrap();
        assert_eq!(found.directory_gaps, vec![19]);
        assert_eq!(found.recovered_files.len(), 1);
        assert_eq!(found.recovered_files[0].path, "AFTER.TXT");
    }
    #[test]
    fn bad_file_data_never_exported_or_zero_filled_but_other_files_survive() {
        let mut img = image();
        file(&mut img, 19 * 512, b"BAD     TXT", 2, b"damaged");
        file(&mut img, 19 * 512 + 32, b"GOOD    TXT", 3, b"good");
        let found = analyze(&img, &[33]).unwrap();
        assert_eq!(found.recovered_files.len(), 1);
        assert_eq!(found.recovered_files[0].path, "GOOD.TXT");
        assert!(found.skipped[0].reason.contains("unreadable"));
    }
    #[test]
    fn intact_fat_mirror_can_supply_missing_entry_but_conflicts_refused() {
        let mut img = image();
        file(&mut img, 19 * 512, b"MIRROR  TXT", 2, b"good");
        let found = analyze(&img, &[1]).unwrap();
        assert_eq!(found.recovered_files[0].fat_links[0].copies, vec![1]);
        set_fat(&mut img, 0, 2, 0);
        let conflict = analyze(&img, &[]).unwrap();
        assert!(conflict.recovered_files.is_empty());
        assert!(conflict.skipped[0].reason.contains("disagree"));
        assert!(analyze(&img, &[1, 10]).unwrap().recovered_files.is_empty());
    }
    #[test]
    fn fat_entry_straddling_sector_uses_both_known_bytes() {
        let mut img = image();
        file(&mut img, 19 * 512, b"BOUNDARYTXT", 341, b"good");
        let found = analyze(&img, &[2, 10]).unwrap();
        assert!(found.recovered_files.is_empty()); // offset 511 needs FAT sectors 1+2 or 10+11
        let intact = analyze(&img, &[2]).unwrap();
        assert_eq!(intact.recovered_files[0].fat_links[0].copies, vec![1]);
    }
    #[test]
    fn cycles_short_chains_and_crosslinked_files_are_refused() {
        let mut img = image();
        file(&mut img, 19 * 512, b"FIRST   TXT", 2, b"good");
        entry(&mut img, 19 * 512 + 32, b"SECOND  TXT", 2, 4, false);
        assert!(analyze(&img, &[]).unwrap().recovered_files.is_empty());
        assert_eq!(analyze(&img, &[]).unwrap().crosslinked_clusters, vec![2]);
        img[19 * 512 + 32..19 * 512 + 64].fill(0);
        for copy in 0..2 {
            set_fat(&mut img, copy, 2, 2);
        }
        assert!(
            analyze(&img, &[]).unwrap().skipped[0]
                .reason
                .contains("cycle")
        );
        for copy in 0..2 {
            set_fat(&mut img, copy, 2, 0xfff);
        }
        entry(&mut img, 19 * 512, b"FIRST   TXT", 2, 600, false);
        assert!(
            analyze(&img, &[]).unwrap().skipped[0]
                .reason
                .contains("size")
        );
    }
    #[test]
    fn subdirectory_prefix_recovery_and_directory_file_crosslink_guard() {
        let mut img = image();
        entry(&mut img, 19 * 512, b"FOLDER     ", 2, 0, true);
        for copy in 0..2 {
            set_fat(&mut img, copy, 2, 0xfff);
        }
        file(&mut img, 33 * 512, b"CHILD   TXT", 3, b"child");
        let found = analyze(&img, &[]).unwrap();
        assert_eq!(found.recovered_files[0].path, "FOLDER/CHILD.TXT");
        assert!(found.recovered_files[0].metadata_lbas.contains(&33));
        entry(&mut img, 19 * 512 + 32, b"CLASH   TXT", 2, 4, false);
        assert!(analyze(&img, &[]).unwrap().recovered_files.is_empty());
    }
    #[test]
    fn corrupt_boot_length_and_sector_evidence_fail_closed() {
        let img = image();
        assert!(analyze(&img, &[0]).is_err());
        assert!(analyze(&img, &[24, 24]).is_err());
        assert!(analyze(&img, &[2880]).is_err());
        assert!(analyze(&img[..img.len() - 512], &[]).is_err());
        let mut invalid = img.clone();
        invalid[13] = 3;
        assert!(analyze(&invalid, &[]).is_err());
    }
    #[test]
    fn hundreds_of_bad_sectors_do_not_hide_an_intact_file_or_invent_missing_names() {
        let mut img = image();
        file(&mut img, 19 * 512, b"INTACT  TXT", 2, b"intact bytes");
        file(&mut img, 19 * 512 + 32, b"DAMAGED TXT", 3, b"lost bytes");
        let bad = std::iter::once(34).chain(100..800).collect::<Vec<_>>();
        let found = analyze(&img, &bad).unwrap();
        assert_eq!(found.bad_lbas.len(), 701);
        assert_eq!(found.recovered_files.len(), 1);
        assert_eq!(file_bytes(&img, &found.recovered_files[0]), b"intact bytes");
        assert_eq!(found.skipped.len(), 1);
        let missing_directory = analyze(&img, &[19]).unwrap();
        assert!(missing_directory.recovered_files.is_empty());
        assert_eq!(missing_directory.directory_gaps, vec![19]);
    }
    #[test]
    fn deleted_long_name_and_device_names_never_become_unsafe_paths() {
        let mut img = image();
        img[19 * 512] = 0xe5;
        img[19 * 512 + 32] = 0x41;
        img[19 * 512 + 43] = 0x0f;
        file(&mut img, 19 * 512 + 64, b"CON     TXT", 2, b"safe");
        let found = analyze(&img, &[]).unwrap();
        assert_eq!(found.deleted_entries_not_recovered, 1);
        assert_eq!(found.long_name_entries_not_used, 1);
        assert!(found.recovered_files[0].path.starts_with("_FAT_"));
        assert!(!short_name(b"../     TXT").contains('/'));
        assert_eq!(short_name(b"SET_UP  EX_"), "SET_UP.EX_");
        assert_ne!(short_name(b"_x2F    TXT"), short_name(b"/       TXT"));
        assert_ne!(
            short_name(b"_X2F    TXT").to_lowercase(),
            short_name(b"/       TXT").to_lowercase()
        );
    }
}
