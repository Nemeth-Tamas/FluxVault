//! Explicit forensic recovery of deleted entries; never part of live delivery.
use super::*;
use crate::carving::{self, Region};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletedEntry {
    pub directory_entry_offset: usize,
    pub raw_entry: [u8; 32],
    /// Context only. The erased first short-name byte/LFN order is not guessed.
    pub parent_directory_hint: String,
    pub ancestor_clusters: Vec<u16>,
    pub metadata_lbas: Vec<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeletedAnalysis {
    pub engine_version: u32,
    pub entries: Vec<DeletedEntry>,
    /// Surviving terminated allocation, not a claim of historical authenticity.
    pub surviving_chain_files: Vec<FileRecord>,
    pub skipped: Vec<Issue>,
    pub contiguous_hypotheses: usize,
    pub original_names_known: bool,
    pub customer_delivery_certified: bool,
    pub warning: String,
}

struct Candidate {
    entry: DeletedEntry,
    clusters: Vec<u16>,
    links: Vec<FatLink>,
    free: bool,
    error: Option<String>,
}

pub(crate) fn analyze_deleted(
    image: &[u8],
    analysis: &Analysis,
) -> Result<(DeletedAnalysis, carving::Analysis), String> {
    let layout = &analysis.layout;
    let bad = analysis.bad_lbas.iter().copied().collect::<BTreeSet<_>>();
    let owned = analysis
        .owned_clusters
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let crosslinked = analysis
        .crosslinked_clusters
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut incoming = BTreeMap::<u16, Vec<u16>>::new();
    for cluster in 2..(layout.clusters + 2) as u16 {
        if let Ok(link) = layout.link(image, &bad, cluster)
            && layout.valid_cluster(link.next)
        {
            incoming.entry(link.next).or_default().push(cluster);
        }
    }
    let mut result = DeletedAnalysis {
        engine_version: 1,
        entries: analysis.deleted_entries.clone(),
        warning: "Opt-in forensic candidates only, excluded from default live delivery. Deleted names are reconstructed; original first character and deleted LFN order are lost. Surviving allocation does not prove original content. Free-cluster contiguity is a hypothesis and requires independent signature validation. Deleted directories are not reconstructed; only entries in reachable readable directories are searched. Unknown bytes are never guessed or joined.".into(),
        ..DeletedAnalysis::default()
    };
    let mut candidates = Vec::new();
    let mut claims = BTreeMap::<u16, usize>::new();
    let mut work = 0usize;
    for entry in &analysis.deleted_entries {
        let raw = &entry.raw_entry;
        let start = word(raw, 26);
        let size = dword(raw, 28) as usize;
        let chain = layout.chain(image, &bad, start);
        let mut candidate = Candidate {
            entry: entry.clone(),
            clusters: chain.clusters,
            links: chain.links,
            free: false,
            error: chain.error,
        };
        // A zero FAT entry loses the original chain. Only an all-free contiguous
        // extent is considered, explicitly hypothetical, never as an intact file.
        if size > 0
            && size <= image.len()
            && layout.valid_cluster(start)
            && candidate.links.first().is_some_and(|l| l.next == 0)
            && raw[11] & 0x10 == 0
        {
            let count = size.div_ceil(layout.sectors_per_cluster * SECTOR);
            let end = start as usize + count;
            if end <= layout.clusters + 2 {
                candidate.free = true;
                candidate.error = None;
                candidate.clusters = (start as usize..end).map(|c| c as u16).collect();
                candidate.links.clear();
                for &cluster in &candidate.clusters {
                    match layout.link(image, &bad, cluster) {
                        Ok(link) if link.next == 0 => candidate.links.push(link),
                        Ok(_) => {
                            candidate.error = Some(
                                "Contiguous hypothesis intersects reallocated/non-free clusters"
                                    .into(),
                            );
                            break;
                        }
                        Err(e) => {
                            candidate.error = Some(e);
                            break;
                        }
                    }
                }
            }
        }
        for &cluster in &candidate.clusters {
            *claims.entry(cluster).or_default() += 1;
        }
        work += candidate.clusters.len();
        if work > 65_536 {
            return Err("Deleted candidate allocation work ceiling reached".into());
        }
        candidates.push(candidate);
    }
    let mut regions = Vec::new();
    for candidate in candidates {
        let entry = &candidate.entry;
        let raw = &entry.raw_entry;
        let size = dword(raw, 28) as usize;
        let name = format!(
            "DeletedRecovery/deleted_{:08x}",
            entry.directory_entry_offset
        );
        let reason = if raw[0] != 0xe5 || raw[11] & 0xd8 != 0 || word(raw, 20) != 0 {
            Some("Deleted directory or malformed/unsupported attributes; no reconstruction".into())
        } else if size == 0 || size > image.len() {
            Some("Empty/oversized deleted entry has no bounded payload".into())
        } else if entry
            .ancestor_clusters
            .iter()
            .any(|c| crosslinked.contains(c))
        {
            Some("Deleted entry belongs to an ambiguous directory chain".into())
        } else if candidate.clusters.iter().any(|c| owned.contains(c)) {
            Some("Deleted allocation intersects reachable live ownership; possible reuse".into())
        } else if candidate.clusters.iter().any(|c| claims[c] > 1) {
            Some("Deleted entries overlap; historical ownership is ambiguous".into())
        } else if candidate.clusters.iter().enumerate().any(|(i, c)| {
            let expected = if candidate.free || i == 0 {
                None
            } else {
                Some(candidate.clusters[i - 1])
            };
            incoming
                .get(c)
                .is_some_and(|sources| sources.len() != 1 || Some(sources[0]) != expected)
        }) {
            Some("Deleted allocation has an external/shared incoming FAT link".into())
        } else if candidate.clusters.len() != size.div_ceil(layout.sectors_per_cluster * SECTOR) {
            Some("Deleted size and allocation extent disagree".into())
        } else {
            candidate.error
        };
        if let Some(reason) = reason {
            result.skipped.push(Issue { path: name, reason });
            continue;
        }
        let lbas = candidate
            .clusters
            .iter()
            .flat_map(|c| {
                let start = layout.cluster_start(*c);
                start..start + layout.sectors_per_cluster
            })
            .take(size.div_ceil(SECTOR))
            .map(|l| l as u64)
            .collect::<Vec<_>>();
        if lbas.iter().any(|l| bad.contains(l)) {
            result.skipped.push(Issue {
                path: name,
                reason:
                    "Deleted candidate intersects unreadable/conflicting sectors; no holes joined"
                        .into(),
            });
            continue;
        }
        let mut metadata = entry.metadata_lbas.iter().copied().collect::<BTreeSet<_>>();
        metadata.extend(
            candidate
                .links
                .iter()
                .flat_map(|l| l.source_lbas.iter().copied()),
        );
        if candidate.free {
            result.contiguous_hypotheses += 1;
            regions.push(Region {
                lbas,
                method: "deleted_entry_free_contiguous_hypothesis".into(),
                allocation_scope: "deleted_entry_association_hypothesis_original_content_unknown"
                    .into(),
                fat_links: candidate.links,
                metadata_lbas: metadata.into_iter().collect(),
                byte_length: Some(size),
                parent_file: Some(name),
            });
        } else {
            // Never use the damaged original alias as a filesystem path. A safe
            // ASCII extension is a hint only, not a type/content validation.
            let extension = raw[8..11]
                .iter()
                .copied()
                .take_while(|c| *c != b' ')
                .collect::<Vec<_>>();
            let extension =
                if !extension.is_empty() && extension.iter().all(u8::is_ascii_alphanumeric) {
                    String::from_utf8(extension).unwrap().to_ascii_lowercase()
                } else {
                    "bin".into()
                };
            let mut record = FileRecord {
                path: format!("{name}.{extension}"),
                short_name_hex: hex(&raw[..11]),
                short_name_case_flags: raw[12],
                name_method:
                    "deleted directory entry; offset name reconstructed, extension only a hint"
                        .into(),
                long_name: None,
                bytes: size,
                sha256: String::new(),
                directory_entry_offset: entry.directory_entry_offset,
                metadata_lbas: metadata.into_iter().collect(),
                data_lbas: lbas,
                clusters: candidate.clusters,
                fat_links: candidate.links,
                modified_dos_date: word(raw, 24),
                modified_dos_time: word(raw, 22),
            };
            record.sha256 = format!("{:x}", Sha256::digest(file_bytes(image, &record)));
            result.surviving_chain_files.push(record);
        }
    }
    let hashes = analysis
        .recovered_files
        .iter()
        .chain(&result.surviving_chain_files)
        .map(|f| f.sha256.clone())
        .collect::<Vec<_>>();
    let mut carved = carving::analyze(image, &analysis.bad_lbas, &regions, &hashes)?;
    for file in &mut carved.files {
        file.path = file
            .path
            .replacen("SignatureRecovery/", "DeletedRecovery/", 1);
    }
    carved.allocation_warning = Some(result.warning.clone());
    Ok((result, carved))
}

#[cfg(test)]
mod tests {
    use super::super::tests::{file, image, set_fat};
    use super::*;

    fn deleted(image: &mut [u8], at: usize, start: u16, bytes: &[u8]) {
        file(image, at, b"REMOVED TXT", start, bytes);
        image[at] = 0xe5;
    }
    fn recover(image: &[u8], bad: &[u64]) -> (DeletedAnalysis, carving::Analysis) {
        analyze_deleted(image, &super::super::analyze(image, bad).unwrap()).unwrap()
    }

    #[test]
    fn surviving_fragmented_chain_retains_exact_evidence_without_original_name_claim() {
        let mut img = image();
        let root = 19 * 512;
        let payload = vec![b'X'; 700];
        deleted(&mut img, root, 2, &payload[..512]);
        img[root + 28..root + 32].copy_from_slice(&700u32.to_le_bytes());
        img[36 * 512..36 * 512 + 188].copy_from_slice(&payload[512..]);
        for copy in 0..2 {
            set_fat(&mut img, copy, 2, 5);
            set_fat(&mut img, copy, 5, 0xfff);
        }
        let normal = super::super::analyze(&img, &[]).unwrap();
        assert!(normal.recovered_files.is_empty());
        let (result, carved) = analyze_deleted(&img, &normal).unwrap();
        assert_eq!(result.surviving_chain_files.len(), 1);
        let record = &result.surviving_chain_files[0];
        assert_eq!(file_bytes(&img, record), payload);
        assert_eq!(record.data_lbas, vec![33, 36]);
        assert!(record.path.starts_with("DeletedRecovery/deleted_"));
        assert!(!result.original_names_known);
        assert!(!result.customer_delivery_certified);
        assert!(carved.files.is_empty());
        assert_eq!(result.entries[0].raw_entry[0], 0xe5);
    }

    #[test]
    fn freed_contiguity_is_only_a_validated_candidate_not_a_complete_deleted_file() {
        let mut img = image();
        let payload = b"{\\rtf1 a deleted candidate}";
        deleted(&mut img, 19 * 512, 2, payload);
        for copy in 0..2 {
            set_fat(&mut img, copy, 2, 0);
        }
        let (result, carved) = recover(&img, &[]);
        assert_eq!(result.contiguous_hypotheses, 1);
        assert!(result.surviving_chain_files.is_empty());
        assert_eq!(carved.files.len(), 1);
        assert_eq!(
            carving::file_bytes(&img, &carved.files[0]).unwrap(),
            payload
        );
        assert!(carved.files[0].allocation_scope.contains("hypothesis"));
        assert!(
            carved.files[0]
                .parent_file
                .as_ref()
                .unwrap()
                .starts_with("DeletedRecovery/")
        );
        img[33 * 512..33 * 512 + payload.len()].fill(b'x');
        assert!(recover(&img, &[]).1.files.is_empty());
    }

    #[test]
    fn live_reuse_overlapping_deleted_entries_bad_bytes_and_fat_disagreement_are_refused() {
        for scenario in 0..4 {
            let mut img = image();
            deleted(&mut img, 19 * 512, 2, b"original");
            let bad = if scenario == 2 { vec![33] } else { vec![] };
            match scenario {
                0 => file(&mut img, 19 * 512 + 32, b"LIVE    TXT", 2, b"new data"),
                1 => deleted(&mut img, 19 * 512 + 32, 2, b"other"),
                3 => set_fat(&mut img, 1, 2, 0),
                _ => (),
            }
            let (result, carved) = recover(&img, &bad);
            assert!(
                result.surviving_chain_files.is_empty(),
                "scenario {scenario}"
            );
            assert!(carved.files.is_empty());
            assert!(!result.skipped.is_empty());
        }
    }

    #[test]
    fn deleted_directories_lfn_slots_unsafe_aliases_and_external_links_are_not_guessed() {
        let mut img = image();
        deleted(&mut img, 19 * 512, 2, b"data");
        img[19 * 512 + 8..19 * 512 + 11].copy_from_slice(b"../");
        let (result, _) = recover(&img, &[]);
        assert!(result.surviving_chain_files[0].path.ends_with(".bin"));
        img[19 * 512 + 11] = 0x10;
        assert!(recover(&img, &[]).0.surviving_chain_files.is_empty());
        img[19 * 512 + 11] = 0x0f;
        assert!(recover(&img, &[]).0.entries.is_empty());
        img[19 * 512 + 11] = 0;
        for copy in 0..2 {
            set_fat(&mut img, copy, 4, 2);
        }
        assert!(recover(&img, &[]).0.surviving_chain_files.is_empty());
    }

    #[test]
    fn freed_hypotheses_stop_at_reallocation_and_declared_eof() {
        let mut img = image();
        deleted(&mut img, 19 * 512, 2, b"{\\rtf1 truncated");
        img[33 * 512 + 100] = b'}'; // outside declared EOF: cannot complete candidate
        for copy in 0..2 {
            set_fat(&mut img, copy, 2, 0);
        }
        assert!(recover(&img, &[]).1.files.is_empty());
        img[19 * 512 + 28..19 * 512 + 32].copy_from_slice(&700u32.to_le_bytes());
        for copy in 0..2 {
            set_fat(&mut img, copy, 3, 0xfff);
        }
        assert!(recover(&img, &[]).1.files.is_empty());
    }
}
