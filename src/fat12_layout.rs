//! Missing-BPB fallback for two explicitly supported standard floppy hypotheses.
//! Never edits the image or treats inferred layout values as physically read bytes.
//! FAT12 packing/entry semantics: Microsoft FAT Specification, sections 3--6:
//! https://www.scs.stanford.edu/~zyedidia/docs/_other/fat.pdf

use super::{Layout, LayoutEvidence, MAX_IMAGE_BYTES, SECTOR, dword, word};
use std::collections::BTreeSet;

pub(super) fn resolve(
    image: &[u8],
    bad: &BTreeSet<u64>,
) -> Result<(Layout, LayoutEvidence), String> {
    let standard = standard(image, bad);
    if standard
        .as_ref()
        .is_ok_and(|(_, evidence)| evidence.warning.is_none())
    {
        return standard;
    }
    // Readable contradictory/partitioned BPBs are never overridden. Broader
    // inference is admitted only for an acquisition-excluded boot sector.
    if standard.is_err() && !bad.contains(&0) {
        return standard;
    }
    let competition = super::layout_candidates::inspect(image, bad)?;
    match standard {
        Ok((layout, mut evidence)) => {
            if let Some(index) = competition.selected_candidate {
                let other = &competition.candidates[index];
                if other.layout.data_start != layout.data_start
                    || other.layout.sectors_per_cluster != layout.sectors_per_cluster
                    || other.layout.root_start != layout.root_start
                {
                    return content_supported(competition, evidence.boot_error.unwrap_or_default());
                }
            } else if competition.candidates.iter().any(|c| {
                c.content_anchors.len() >= 2
                    && (c.layout.data_start != layout.data_start
                        || c.layout.sectors_per_cluster != layout.sectors_per_cluster
                        || c.layout.root_start != layout.root_start)
            }) {
                return Err("Competing content-supported layout mappings or a bounded-out search; no automatic winner".into());
            }
            evidence.competition = Some(competition);
            Ok((layout, evidence))
        }
        Err(error) => {
            if competition.selected_candidate.is_none() {
                return Err(format!(
                    "{error}; bounded competing-layout comparison found {} plausible candidates, no unique two-content-anchor candidate (search complete: {})",
                    competition.candidates.len(),
                    competition.search_complete_within_bounds
                ));
            }
            content_supported(competition, error)
        }
    }
}

fn content_supported(
    competition: super::LayoutCompetition,
    error: String,
) -> Result<(Layout, LayoutEvidence), String> {
    let index = competition
        .selected_candidate
        .ok_or("No selected content-supported layout")?;
    let candidate = &competition.candidates[index];
    let layout = candidate.layout.clone();
    let evidence = LayoutEvidence {
                method: "inferred_content_supported_layout".into(),
                standard_profile: None,
                boot_error: Some(error),
                source_lbas: (layout.reserved_sectors..layout.data_start).map(|n|n as u64).collect(),
                anchor_directory_offsets: candidate.content_anchors.iter().map(|a|a.directory_offset).collect(),
                warning: Some("Bounded missing-boot hypothesis supported by mirrored FAT/root chains and at least two structurally validated, disjoint file contents. Lower-evidence alternatives remain recorded. No boot bytes reconstructed, unrestricted layout uniqueness or original semantics certified.".into()),
                competition: Some(competition),
    };
    Ok((layout, evidence))
}

fn standard(image: &[u8], bad: &BTreeSet<u64>) -> Result<(Layout, LayoutEvidence), String> {
    let error = match Layout::parse(image, bad) {
        Ok(layout) => return Ok((layout, LayoutEvidence::default())),
        Err(error) => error,
    };
    if image.len() < SECTOR || image.len() > MAX_IMAGE_BYTES || !image.len().is_multiple_of(SECTOR)
    {
        return Err(error);
    }
    // A present, signed 512-byte BPB with inconsistent/unsupported fields is not
    // overridden. In particular, do not reinterpret partitioned/truncated images.
    if !bad.contains(&0) && word(image, 11) == 512 && image[510..512] == [0x55, 0xaa] {
        return Err(error);
    }
    let (profile, spc, spf, roots, media) = match image.len() / SECTOR {
        1440 => ("ibm.720", 2, 3, 112, 0xf9),
        2880 => ("ibm.1440", 1, 9, 224, 0xf0),
        _ => return Err(format!("{error}; no supported standard-layout hypothesis")),
    };
    // A lost boot signature alone must not silently replace a surviving custom
    // layout. If the BPB still identifies 512-byte sectors, every layout field
    // must corroborate this candidate (including the absence of a partition).
    if !bad.contains(&0)
        && word(image, 11) == 512
        && (image[13] as usize != spc
            || word(image, 14) != 1
            || image[16] != 2
            || word(image, 17) as usize != roots
            || word(image, 19) as usize != image.len() / SECTOR
            || image[21] != media
            || word(image, 22) as usize != spf
            || dword(image, 28) != 0)
    {
        return Err(format!(
            "{error}; surviving BPB fields contradict the standard hypothesis"
        ));
    }
    let root_start = 1 + 2 * spf;
    let data_start = root_start + (roots * 32usize).div_ceil(SECTOR);
    let layout = Layout {
        total_sectors: image.len() / SECTOR,
        sectors_per_cluster: spc,
        reserved_sectors: 1,
        fat_copies: 2,
        sectors_per_fat: spf,
        root_entries: roots,
        root_start,
        data_start,
        clusters: (image.len() / SECTOR - data_start) / spc,
    };
    let refused = |why: &str| format!("{error}; standard-layout inference refused: {why}");
    // Require BOTH entire FATs and the root region, not a guessed location from
    // image length alone, a majority vote, or placeholder-filled metadata.
    let source_lbas = (1..data_start).map(|lba| lba as u64).collect::<Vec<_>>();
    if source_lbas.iter().any(|lba| bad.contains(lba)) {
        return Err(refused("unreadable FAT/root metadata"));
    }
    let first = &image[SECTOR..(1 + spf) * SECTOR];
    let second = &image[(1 + spf) * SECTOR..root_start * SECTOR];
    if first != second || first[..3] != [media, 0xff, 0xff] {
        return Err(refused(
            "FAT headers/copies disagree with the standard hypothesis",
        ));
    }
    for cluster in 2..layout.clusters + 2 {
        let packed = word(first, cluster * 3 / 2);
        let next = if cluster.is_multiple_of(2) {
            packed & 0xfff
        } else {
            packed >> 4
        };
        if next != 0 && next != 0xff7 && next < 0xff8 && !layout.valid_cluster(next) {
            return Err(refused("invalid allocation value in surviving FATs"));
        }
    }
    let mut anchors = Vec::new();
    let mut end_seen = false;
    for at in (root_start * SECTOR..data_start * SECTOR).step_by(32) {
        let entry = &image[at..at + 32];
        if entry[0] == 0 {
            end_seen = true;
            break;
        }
        if entry[0] == 0xe5 || entry[11] == 0x0f {
            continue;
        }
        if entry[11] & 0xc0 != 0
            || word(entry, 20) != 0
            || entry[..11].iter().any(|b| *b < 0x20)
            || entry[..8].iter().all(|b| *b == b' ')
        {
            return Err(refused("implausible active root-directory entry"));
        }
        if entry[11] & 0x18 != 0 {
            continue; // Directory/volume labels are not file-size anchors.
        }
        let size = dword(entry, 28) as usize;
        if size == 0 {
            continue;
        }
        if size > image.len() {
            return Err(refused("root file length exceeds image"));
        }
        let chain = layout.chain(image, bad, word(entry, 26));
        if chain.error.is_none() && chain.clusters.len() == size.div_ceil(spc * SECTOR) {
            anchors.push(at);
        }
    }
    if !end_seen || anchors.is_empty() {
        return Err(refused(
            "no intact root end marker and size-consistent live file chain",
        ));
    }
    Ok((layout, LayoutEvidence {
        method: "inferred_standard_layout".into(),
        standard_profile: Some(profile.into()),
        boot_error: Some(error),
        source_lbas,
        anchor_directory_offsets: anchors,
        warning: Some("Standard-layout hypothesis corroborated by matching readable FATs and root file sizes/chains. Nonstandard layouts of the same image length are not exhaustively excluded. Boot bytes are NOT reconstructed; inferred-layout files require recovery review, not certified delivery.".into()),
        competition: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fat12::{
        analyze, file_bytes,
        tests::{file, image, set_fat},
    };

    #[test]
    fn absent_boot_recovers_intact_file_without_modifying_image_or_claiming_read_boot() {
        let mut img = image();
        file(&mut img, 19 * SECTOR, b"INTACT  TXT", 2, b"known bytes");
        img[..SECTOR].fill(0);
        let before = img.clone();
        let found = analyze(&img, &[0]).unwrap();
        assert_eq!(file_bytes(&img, &found.recovered_files[0]), b"known bytes");
        assert_eq!(found.layout_evidence.method, "inferred_standard_layout");
        assert_eq!(
            found.layout_evidence.standard_profile.as_deref(),
            Some("ibm.1440")
        );
        assert_eq!(
            found.layout_evidence.anchor_directory_offsets,
            vec![19 * SECTOR]
        );
        assert!(!found.recovered_files[0].metadata_lbas.contains(&0));
        assert!(found.layout_evidence.warning.is_some());
        assert!(!found.customer_delivery_certified);
        assert_eq!(img, before);
    }

    #[test]
    fn missing_signature_can_infer_but_signed_inconsistent_bpb_is_not_overridden() {
        let mut img = image();
        file(&mut img, 19 * SECTOR, b"GOOD    TXT", 2, b"known bytes");
        img[510..512].fill(0);
        assert!(
            analyze(&img, &[])
                .unwrap()
                .layout_evidence
                .boot_error
                .is_some()
        );
        img[510..512].copy_from_slice(&[0x55, 0xaa]);
        img[13] = 3;
        assert!(analyze(&img, &[]).is_err());
        img[13] = 1;
        img[28] = 1;
        assert!(analyze(&img, &[]).is_err());
        img[510..512].fill(0);
        assert!(analyze(&img, &[]).is_err());
        img[28] = 0;
        img[17..19].copy_from_slice(&240u16.to_le_bytes());
        assert!(analyze(&img, &[]).is_err());
    }

    #[test]
    fn boot_and_hundreds_of_data_gaps_keep_only_complete_files() {
        let mut img = image();
        file(&mut img, 19 * SECTOR, b"GOOD    TXT", 2, b"intact");
        file(&mut img, 19 * SECTOR + 32, b"BAD     TXT", 3, b"lost");
        let bad = [0, 34].into_iter().chain(100..800).collect::<Vec<_>>();
        let found = analyze(&img, &bad).unwrap();
        assert_eq!(found.recovered_files.len(), 1);
        assert_eq!(found.skipped.len(), 1);
        assert_eq!(found.recovered_files[0].path, "GOOD.TXT");
    }

    #[test]
    fn size_alone_empty_deleted_only_or_bad_metadata_cannot_establish_layout() {
        let mut img = image();
        assert!(analyze(&img, &[0]).is_err());
        file(&mut img, 19 * SECTOR, b"GOOD    TXT", 2, b"known");
        for lba in [1, 9, 10, 18, 19, 32] {
            assert!(analyze(&img, &[0, lba]).is_err());
        }
        img[19 * SECTOR] = 0xe5;
        assert!(analyze(&img, &[0]).is_err());
    }

    #[test]
    fn conflicting_invalid_looped_and_length_inconsistent_fats_are_refused() {
        for next in [1, 2, 0xff0, 0xff7, 3000] {
            let mut img = image();
            file(&mut img, 19 * SECTOR, b"GOOD    TXT", 2, b"known");
            for copy in 0..2 {
                set_fat(&mut img, copy, 2, next);
            }
            assert!(analyze(&img, &[0]).is_err(), "link {next}");
        }
        let mut img = image();
        file(&mut img, 19 * SECTOR, b"GOOD    TXT", 2, b"known");
        set_fat(&mut img, 1, 2, 0xff8);
        assert!(analyze(&img, &[0]).is_err());
    }

    #[test]
    fn standard_dd_layout_and_fragmented_chain_are_correlated() {
        let mut img = vec![0; 1440 * SECTOR];
        for sector in [1, 4] {
            img[sector * SECTOR..sector * SECTOR + 6]
                .copy_from_slice(&[0xf9, 0xff, 0xff, 4, 0xf0, 0xff]);
            let at = sector * SECTOR + 6; // cluster 4 -> EOC
            img[at..at + 2].copy_from_slice(&[0xff, 0x0f]);
        }
        crate::fat12::tests::entry(&mut img, 7 * SECTOR, b"FRAG    TXT", 2, 1500, false);
        img[14 * SECTOR..16 * SECTOR].fill(b'A');
        img[18 * SECTOR..19 * SECTOR].fill(b'B');
        let found = analyze(&img, &[0]).unwrap();
        assert_eq!(found.layout.data_start, 14);
        assert_eq!(found.layout.sectors_per_cluster, 2);
        assert_eq!(found.recovered_files[0].clusters, vec![2, 4]);
        assert_eq!(
            file_bytes(&img, &found.recovered_files[0]),
            [vec![b'A'; 1024], vec![b'B'; 476]].concat()
        );
    }

    #[test]
    #[ignore = "requires FLUXVAULT_FAT12_TEST_PROJECT; reads saved images only, modifies boot bytes in memory only"]
    fn real_saved_pilot_payloads_survive_simulated_missing_boot_in_memory() {
        let root =
            std::path::PathBuf::from(std::env::var_os("FLUXVAULT_FAT12_TEST_PROJECT").unwrap());
        for disk in [5, 7, 9, 12, 17] {
            let source = root.join(format!("Images/{disk:03}_attempt_001.img"));
            let original = std::fs::read(&source).unwrap();
            let metadata: serde_json::Value = serde_json::from_slice(
                &std::fs::read(root.join(format!("Images/{disk:03}_attempt_001.json"))).unwrap(),
            )
            .unwrap();
            let bad = metadata["bad_sectors"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["lba"].as_u64().unwrap())
                .collect::<Vec<_>>();
            use sha2::Digest;
            assert_eq!(
                format!("{:x}", sha2::Sha256::digest(&original)),
                metadata["sha256"].as_str().unwrap()
            );
            let baseline = analyze(&original, &bad).unwrap();
            let mut damaged = original.clone();
            damaged[..SECTOR].fill(0);
            let bad = [vec![0], bad].concat();
            let recovered = analyze(&damaged, &bad).unwrap();
            let payloads = |analysis: &crate::fat12::Analysis| {
                analysis
                    .recovered_files
                    .iter()
                    .map(|r| {
                        (
                            r.path.clone(),
                            r.sha256.clone(),
                            r.bytes,
                            r.data_lbas.clone(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(payloads(&recovered), payloads(&baseline), "disk {disk:03}");
            assert_eq!(
                recovered.layout_evidence.method,
                if disk == 17 {
                    "inferred_content_supported_layout"
                } else {
                    "inferred_standard_layout"
                }
            );
            if disk == 17 {
                let c = recovered.layout_evidence.competition.as_ref().unwrap();
                assert!(c.search_complete_within_bounds);
                assert_eq!(
                    c.candidates[c.selected_candidate.unwrap()]
                        .content_anchors
                        .len(),
                    14
                );
            }
            assert_eq!(recovered.skipped.len(), baseline.skipped.len());
            assert_eq!(std::fs::read(&source).unwrap(), original);
            eprintln!(
                "Saved {disk:03}: {} identical payload hashes/paths/extents; {} entries skipped; inferred {}",
                recovered.recovered_files.len(),
                recovered.skipped.len(),
                recovered
                    .layout_evidence
                    .standard_profile
                    .as_deref()
                    .unwrap_or("content-supported bounded layout")
            );
        }
    }
}
