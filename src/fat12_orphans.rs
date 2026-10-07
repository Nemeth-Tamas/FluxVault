//! Follow only terminated, unclaimed FAT chains. Never concatenate around holes.
use super::*;
use crate::carving::Region;

pub(crate) fn orphan_regions(
    image: &[u8],
    analysis: &Analysis,
) -> (Vec<Region>, usize, Vec<String>) {
    let layout = &analysis.layout;
    let bad = analysis.bad_lbas.iter().copied().collect::<BTreeSet<_>>();
    let excluded = analysis
        .owned_clusters
        .iter()
        .chain(&analysis.deleted_clusters_excluded)
        .copied()
        .collect::<BTreeSet<_>>();
    let mut links = BTreeMap::new();
    let mut incoming = BTreeMap::<u16, usize>::new();
    let mut issues = vec![];
    for cluster in 2..(layout.clusters + 2) as u16 {
        match layout.link(image, &bad, cluster) {
            Ok(link) => {
                if layout.valid_cluster(link.next) {
                    *incoming.entry(link.next).or_default() += 1;
                }
                if layout.valid_cluster(link.next) || link.next >= 0xff8 {
                    links.insert(cluster, link);
                }
            }
            Err(error) => issues.push(error),
        }
    }
    let heads = links
        .keys()
        .filter(|c| !incoming.contains_key(c))
        .copied()
        .collect::<Vec<_>>();
    let mut visited = BTreeSet::<u16>::new();
    let mut regions: Vec<Region> = vec![];
    let mut chains = 0;
    for head in heads {
        let chain = layout.chain(image, &bad, head);
        visited.extend(chain.clusters.iter().copied());
        if chain.clusters.iter().any(|c| excluded.contains(c)) {
            continue;
        }
        if let Some(error) = chain.error {
            issues.push(format!("Orphan {head}: {error}"));
            continue;
        }
        if chain
            .clusters
            .iter()
            .any(|c| incoming.get(c).copied().unwrap_or(0) > 1)
        {
            issues.push(format!(
                "Orphan {head}: ambiguous shared tail; no bytes exported"
            ));
            continue;
        }
        chains += 1;
        let mut run = vec![];
        let publish = |run: &mut Vec<u64>, regions: &mut Vec<Region>| {
            if !run.is_empty() {
                regions.push(Region {
                    lbas: std::mem::take(run),
                    method: "allocated_orphan_fat_chain_signature".into(),
                    allocation_scope: "allocated_unclaimed_chain_original_live_or_deleted_unknown"
                        .into(),
                    fat_links: chain.links.clone(),
                    metadata_lbas: analysis.layout_evidence.source_lbas.clone(),
                    byte_length: None,
                    parent_file: None,
                });
            }
        };
        for cluster in &chain.clusters {
            let begin = layout.cluster_start(*cluster);
            for lba in begin..begin + layout.sectors_per_cluster {
                if bad.contains(&(lba as u64)) {
                    publish(&mut run, &mut regions);
                } else {
                    run.push(lba as u64);
                }
            }
        }
        publish(&mut run, &mut regions);
    }
    for cluster in links.keys() {
        if !visited.contains(cluster) && !excluded.contains(cluster) {
            issues.push(format!("Unclaimed cluster {cluster}: no unambiguous terminated head (cycle/shared allocation)"));
        }
    }
    (regions, chains, issues)
}

/// Salvage self-contained, validated embedded candidates, never the partial parent.
pub(crate) fn partial_file_regions(analysis: &Analysis) -> Vec<Region> {
    let bad = analysis.bad_lbas.iter().copied().collect::<BTreeSet<_>>();
    let mut regions = vec![];
    for file in &analysis.unrecovered_files {
        let r = &file.record;
        if file.reason.contains("ownership")
            || file.reason.contains("ambiguous")
            || r.clusters
                .iter()
                .any(|c| analysis.crosslinked_clusters.contains(c))
        {
            continue;
        }
        let mut lbas = vec![];
        let mut length = 0;
        let publish = |lbas: &mut Vec<u64>, length: &mut usize, regions: &mut Vec<Region>| {
            if !lbas.is_empty() {
                regions.push(Region {
                    lbas: std::mem::take(lbas),
                    byte_length: Some(std::mem::take(length)),
                    method: "readable_partial_file_embedded_signature".into(),
                    allocation_scope: "live_parent_incomplete_independent_candidate_only".into(),
                    parent_file: Some(r.path.clone()),
                    fat_links: r.fat_links.clone(),
                    metadata_lbas: r.metadata_lbas.clone(),
                });
            }
        };
        for (index, lba) in r.data_lbas.iter().enumerate() {
            let n = r.bytes.saturating_sub(index * SECTOR).min(SECTOR);
            if n == 0 {
                break;
            }
            if bad.contains(lba) {
                publish(&mut lbas, &mut length, &mut regions);
            } else {
                lbas.push(*lba);
                length += n;
            }
        }
        publish(&mut lbas, &mut length, &mut regions);
    }
    regions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::carving;
    fn allocate(image: &mut [u8], cluster: u16, next: u16) {
        for copy in 0..2 {
            crate::fat12::tests::set_fat(image, copy, cluster, next);
        }
    }
    #[test]
    fn fragmented_orphan_recovers_in_logical_order_with_exact_extents() {
        let mut image = crate::fat12::tests::image();
        let mut payload = b"{\\rtf1 ".to_vec();
        payload.extend(vec![b'x'; 600]);
        payload.push(b'}');
        allocate(&mut image, 2, 7);
        allocate(&mut image, 7, 0xfff);
        image[33 * 512..34 * 512].copy_from_slice(&payload[..512]);
        image[38 * 512..38 * 512 + payload.len() - 512].copy_from_slice(&payload[512..]);
        let a = analyze(&image, &[]).unwrap();
        let (regions, chains, issues) = orphan_regions(&image, &a);
        assert_eq!(chains, 1);
        assert!(issues.is_empty());
        let carved = carving::analyze(&image, &[], &regions, &[]).unwrap();
        assert_eq!(carved.files.len(), 1);
        let file = &carved.files[0];
        assert_eq!(file.source_extents.len(), 2);
        assert_eq!(file.source_extents[1].source_byte_offset, 38 * 512);
        assert_eq!(file.fat_links.len(), 2);
        assert_eq!(carving::file_bytes(&image, file).unwrap(), payload);
    }
    #[test]
    fn live_deleted_and_free_clusters_are_not_carved_by_default() {
        let mut image = crate::fat12::tests::image();
        crate::fat12::tests::file(&mut image, 19 * 512, b"LIVE    RTF", 2, b"{\\rtf1 live}");
        crate::fat12::tests::file(
            &mut image,
            19 * 512 + 32,
            b"GONE    RTF",
            3,
            b"{\\rtf1 deleted}",
        );
        image[19 * 512 + 32] = 0xe5;
        image[35 * 512..35 * 512 + 12].copy_from_slice(b"{\\rtf1 free}");
        let a = analyze(&image, &[]).unwrap();
        assert_eq!(a.owned_clusters, vec![2]);
        assert_eq!(a.deleted_clusters_excluded, vec![3]);
        assert!(orphan_regions(&image, &a).0.is_empty());
    }
    #[test]
    fn shared_tails_cycles_and_disagreeing_fats_are_not_guessed() {
        let mut image = crate::fat12::tests::image();
        allocate(&mut image, 2, 4);
        allocate(&mut image, 3, 4);
        allocate(&mut image, 4, 0xfff);
        allocate(&mut image, 5, 6);
        allocate(&mut image, 6, 5);
        allocate(&mut image, 7, 0xfff);
        crate::fat12::tests::set_fat(&mut image, 1, 7, 8);
        let a = analyze(&image, &[]).unwrap();
        let (regions, _, issues) = orphan_regions(&image, &a);
        assert!(regions.is_empty());
        assert!(issues.iter().any(|e| e.contains("shared tail")));
        assert!(issues.iter().any(|e| e.contains("cycle")));
        assert!(issues.iter().any(|e| e.contains("disagree")));
    }
    #[test]
    fn unreadable_orphan_sector_splits_run_instead_of_joining_across_hole() {
        let mut image = crate::fat12::tests::image();
        allocate(&mut image, 2, 3);
        allocate(&mut image, 3, 4);
        allocate(&mut image, 4, 0xfff);
        let a = analyze(&image, &[34]).unwrap();
        let (regions, _, _) = orphan_regions(&image, &a);
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].lbas, vec![33]);
        assert_eq!(regions[1].lbas, vec![35]);
    }
}
