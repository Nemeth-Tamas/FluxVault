//! One missing FAT-link boundary: retain bounded structurally validated suffix
//! hypotheses, not an invented allocation link or an authoritative original.
use super::*;
use crate::carving::{self, Region};

pub(crate) fn hypotheses(
    image: &[u8],
    analysis: &Analysis,
    known_hashes: &[String],
) -> Result<carving::Analysis, String> {
    let layout = &analysis.layout;
    let bad = analysis.bad_lbas.iter().copied().collect::<BTreeSet<_>>();
    let excluded = analysis
        .owned_clusters
        .iter()
        .chain(&analysis.deleted_clusters_excluded)
        .copied()
        .collect::<BTreeSet<_>>();
    let mut incoming = BTreeMap::<u16, usize>::new();
    let mut links = BTreeMap::new();
    for c in 2..(layout.clusters + 2) as u16 {
        if let Ok(link) = layout.link(image, &bad, c) {
            if layout.valid_cluster(link.next) {
                *incoming.entry(link.next).or_default() += 1;
            }
            if layout.valid_cluster(link.next) || link.next >= 0xff8 {
                links.insert(c, link);
            }
        }
    }
    let tails = links
        .keys()
        .filter(|c| !incoming.contains_key(c))
        .map(|c| layout.chain(image, &bad, *c))
        .filter(|chain| {
            chain.error.is_none()
                && !chain
                    .clusters
                    .iter()
                    .any(|c| excluded.contains(c) || incoming.get(c).copied().unwrap_or(0) > 1)
        })
        .collect::<Vec<_>>();
    let mut result = carving::Analysis {
        bad_lbas: analysis.bad_lbas.clone(),
        ..Default::default()
    };
    let mut hashes = known_hashes.iter().cloned().collect::<BTreeSet<_>>();
    let mut trials = 0;
    let mut bytes_checked = 0;
    let mut validation_work = 0;
    let mut exported_bytes = 0;
    for parent in &analysis.unrecovered_files {
        if !partial_file_safe(parent, analysis)
            || parent.unmapped_tail_bytes == 0
            || !parent.reason.contains("unreadable in every copy")
            || !parent.unreadable_ranges.is_empty()
        {
            continue;
        }
        let record = &parent.record;
        let needed = record.bytes.div_ceil(layout.sectors_per_cluster * SECTOR);
        if record.clusters.is_empty() || needed <= record.clusters.len() || needed > layout.clusters
        {
            continue;
        }
        let prefix = record
            .clusters
            .iter()
            .flat_map(|c| {
                let begin = layout.cluster_start(*c);
                (begin..begin + layout.sectors_per_cluster).map(|l| l as u64)
            })
            .collect::<Vec<_>>();
        let mut accepted = 0;
        for tail in &tails {
            if tail.clusters.len() != needed - record.clusters.len() {
                continue;
            }
            if trials >= 64
                || result.probes >= 1024
                || validation_work > 128 * 1024 * 1024
                || bytes_checked + record.bytes > MAX_IMAGE_BYTES * 2
            {
                result.limits_reached = true;
                break;
            }
            let mut lbas = prefix.clone();
            lbas.extend(tail.clusters.iter().flat_map(|c| {
                let begin = layout.cluster_start(*c);
                (begin..begin + layout.sectors_per_cluster).map(|l| l as u64)
            }));
            if lbas.iter().any(|l| bad.contains(l)) {
                continue;
            }
            trials += 1;
            bytes_checked += record.bytes;
            let mut fat_links = record.fat_links.clone();
            fat_links.extend(tail.links.clone());
            let region = Region {
                lbas,
                method: "content_validated_missing_fat_link_hypothesis".into(),
                allocation_scope:
                    "allocated_suffix_original_link_unknown_structural_candidate_not_authoritative"
                        .into(),
                fat_links,
                metadata_lbas: record.metadata_lbas.clone(),
                byte_length: Some(record.bytes),
                parent_file: Some(record.path.clone()),
            };
            let carved = carving::analyze_with_work(
                image,
                &analysis.bad_lbas,
                &[region],
                &[],
                &mut validation_work,
            )?;
            result.limits_reached |= carved.limits_reached;
            result.probes += carved.probes;
            // Accept only a header at the known file start, not an unrelated
            // embedded signature. Format validation can corroborate a candidate
            // but cannot prove that this tail historically belonged to it.
            let start = prefix[0] as usize * SECTOR;
            for mut file in carved.files.into_iter().filter(|f| {
                f.source_extents
                    .first()
                    .is_some_and(|e| e.source_byte_offset == start)
                    && f.bytes > prefix.len() * SECTOR
            }) {
                accepted += 1;
                file.missing_fat_link_cluster = record.clusters.last().copied();
                file.candidate_tail_start_cluster = tail.clusters.first().copied();
                file.path = file
                    .path
                    .replacen("SignatureRecovery/", "FragmentRecovery/", 1);
                if hashes.insert(file.sha256.clone()) {
                    if result.files.len() >= 256 || exported_bytes + file.bytes > MAX_IMAGE_BYTES {
                        result.limits_reached = true;
                        break;
                    }
                    exported_bytes += file.bytes;
                    result.files.push(file);
                }
            }
            let remaining = 1024usize.saturating_sub(result.rejected.len());
            result
                .rejected
                .extend(carved.rejected.into_iter().take(remaining));
        }
        if accepted > 0 {
            result.fragmented_parents_with_candidates += 1;
            if accepted > 1 {
                result.ambiguous_fragmented_parents += 1;
            }
            result.allocation_issues.push(format!("{}: {accepted} structurally validated missing-link hypotheses; all remain non-authoritative, no original allocation/name/content certification",record.path));
        }
        if result.limits_reached {
            break;
        }
    }
    result.fragmented_suffix_trials = trials;
    result.scanned_bytes = bytes_checked;
    if !result.files.is_empty() {
        result.allocation_warning=Some("Missing FAT-link suffix hypotheses are structurally validated alternatives, not proven original chains or repaired files. All matching candidates remain separate; CRC/container validity does not authenticate the historical association.".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fat12::tests as t;
    fn fixture(two: bool) -> (Vec<u8>, Analysis, Vec<u8>) {
        let mut image = t::image();
        let payload = [b"{\\rtf1 ".as_slice(), &vec![b'x'; 600], b"}"].concat();
        t::entry(
            &mut image,
            19 * 512,
            b"BROKEN  RTF",
            2,
            payload.len() as u32,
            false,
        );
        image[33 * 512..34 * 512].copy_from_slice(&payload[..512]);
        for c in if two { vec![350, 351] } else { vec![350] } {
            for copy in 0..2 {
                t::set_fat(&mut image, copy, c, 0xfff);
            }
            let at = (33 + c as usize - 2) * 512;
            image[at..at + payload.len() - 512].copy_from_slice(&payload[512..]);
            if c == 351 {
                image[at] = b'y';
            }
        }
        let a = analyze(&image, &[1, 10]).unwrap();
        (image, a, payload)
    }
    #[test]
    fn missing_link_uses_known_prefix_and_validated_allocated_nonadjacent_suffix() {
        let (image, a, payload) = fixture(false);
        let h = hypotheses(&image, &a, &[]).unwrap();
        assert_eq!(h.files.len(), 1);
        let f = &h.files[0];
        assert!(f.path.starts_with("FragmentRecovery/"));
        assert_eq!(f.missing_fat_link_cluster, Some(2));
        assert_eq!(f.candidate_tail_start_cluster, Some(350));
        assert_eq!(carving::file_bytes(&image, f).unwrap(), payload);
        assert!(!f.customer_delivery_certified);
        assert!(!f.fat_links.iter().any(|l| l.cluster == 2)); // no invented link
    }
    #[test]
    fn competing_valid_tails_are_retained_not_ranked_as_a_true_original() {
        let (image, a, _) = fixture(true);
        let h = hypotheses(&image, &a, &[]).unwrap();
        assert_eq!(h.files.len(), 2);
        assert_eq!(h.ambiguous_fragmented_parents, 1);
        assert_ne!(h.files[0].sha256, h.files[1].sha256);
    }
    #[test]
    fn known_deleted_owned_unreadable_and_structurally_invalid_tails_are_excluded() {
        for mode in 0..4 {
            let (mut image, mut a, _) = fixture(false);
            let at = (33 + 350 - 2) * 512;
            match mode {
                0 => a.deleted_clusters_excluded.push(350),
                1 => a.owned_clusters.push(350),
                2 => a.bad_lbas.push((at / 512) as u64),
                _ => image[at + 95] = b'x',
            };
            assert!(
                hypotheses(&image, &a, &[]).unwrap().files.is_empty(),
                "mode {mode}"
            );
        }
    }

    #[test]
    fn tail_search_is_bounded_and_does_not_relabel_a_prefix_only_object() {
        let (mut image, _, payload) = fixture(false);
        for c in 352..450 {
            for copy in 0..2 {
                t::set_fat(&mut image, copy, c, 0xfff);
            }
            let at = (33 + c as usize - 2) * SECTOR;
            image[at..at + payload.len() - SECTOR].copy_from_slice(&payload[SECTOR..]);
        }
        let a = analyze(&image, &[1, 10]).unwrap();
        let h = hypotheses(&image, &a, &[]).unwrap();
        assert_eq!(h.fragmented_suffix_trials, 64);
        assert!(h.limits_reached);
        assert!(h.files.iter().all(|f| f.bytes > SECTOR));
        image[33 * SECTOR..34 * SECTOR].fill(b' ');
        image[33 * SECTOR..33 * SECTOR + 9].copy_from_slice(b"{\\rtf1 x}");
        let a = analyze(&image, &[1, 10]).unwrap();
        let h = hypotheses(&image, &a, &[]).unwrap();
        assert!(h.files.is_empty());
        assert_eq!(h.fragmented_parents_with_candidates, 0);
    }
}
