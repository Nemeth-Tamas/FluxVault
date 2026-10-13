//! Bounded FAT12 layout comparison, never a guessed/repaired boot sector.
//! Layout arithmetic: Microsoft FAT Specification §§3–6 (see fat12_layout.rs).
//! Policy bounds are deliberately not an unrestricted uniqueness certificate.
use super::{Layout, MAX_IMAGE_BYTES, SECTOR, dword, word};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const MAX_TRIALS: usize = 8192;
const MAX_CANDIDATES: usize = 512;
const MAX_WORK: u64 = 128 * 1024 * 1024;
#[derive(Clone, Copy)]
pub(super) struct Budget {
    pub trials: usize,
    pub candidates: usize,
    pub work: u64,
}
#[cfg(test)]
#[path = "fat12_layout_candidates_tests.rs"]
mod tests;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentAnchor {
    pub directory_offset: usize,
    pub declared_bytes: usize,
    pub payload_sha256: String,
    pub source_lbas: Vec<u64>,
    pub validation: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutCandidate {
    pub layout: Layout,
    pub chain_anchors: usize,
    pub content_anchors: Vec<ContentAnchor>,
    pub confidence: String,
    pub original_layout_certified: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutCompetition {
    pub schema_version: u32,
    pub image_sha256: String,
    pub excluded_lbas: Vec<u64>,
    pub tested_layouts: usize,
    pub search_complete_within_bounds: bool,
    pub maximum_trials: usize,
    pub maximum_candidates: usize,
    pub maximum_validation_work_bytes: u64,
    pub charged_validation_work_bytes: u64,
    pub selected_candidate: Option<usize>,
    pub candidates: Vec<LayoutCandidate>,
    pub limits: String,
}

pub(super) fn inspect(image: &[u8], bad: &BTreeSet<u64>) -> Result<LayoutCompetition, String> {
    inspect_bounded(
        image,
        bad,
        Budget {
            trials: MAX_TRIALS,
            candidates: MAX_CANDIDATES,
            work: MAX_WORK,
        },
    )
}
pub(super) fn inspect_bounded(
    image: &[u8],
    bad: &BTreeSet<u64>,
    budget: Budget,
) -> Result<LayoutCompetition, String> {
    if image.len() < SECTOR || image.len() > MAX_IMAGE_BYTES || !image.len().is_multiple_of(SECTOR)
    {
        return Err("Layout comparison requires a bounded whole-sector floppy image".into());
    }
    let mut result = LayoutCompetition {
        schema_version:1, image_sha256:format!("{:x}",Sha256::digest(image)),
        excluded_lbas:bad.iter().copied().collect(), tested_layouts:0,
        search_complete_within_bounds:true, maximum_trials:budget.trials, maximum_candidates:budget.candidates,
        maximum_validation_work_bytes:budget.work, charged_validation_work_bytes:0, selected_candidate:None, candidates:vec![],
        limits:"Two fully readable identical FATs; reserved 1..32; FAT sectors 1..32; root entries 16..1024 in multiples of 16; cluster sectors 1,2,4,8,16,32,64; trial/candidate/work ceilings recorded numerically. This finite family does not exclude arbitrary nonstandard, partitioned or single-FAT layouts. Structural preference is not authority to override a readable boot sector or proof of original content/ownership; no boot bytes written or customer certification.".into(),
    };
    let total = image.len() / SECTOR;
    let mut work = 0;
    let mut cache: BTreeMap<String, Result<Option<String>, String>> = BTreeMap::new();
    'search: for reserved in 1usize..=32 {
        if reserved >= total || bad.contains(&(reserved as u64)) {
            continue;
        }
        let at = reserved * SECTOR;
        if !matches!(image[at], 0xf0 | 0xf8..=0xff) || image[at + 1..at + 3] != [0xff, 0xff] {
            continue;
        }
        for spf in 1usize..=32 {
            let root = reserved + 2 * spf;
            if root >= total || (reserved..root).any(|n| bad.contains(&(n as u64))) {
                continue;
            }
            let first = &image[at..(reserved + spf) * SECTOR];
            if first != &image[(reserved + spf) * SECTOR..root * SECTOR] {
                continue;
            }
            for roots in (16usize..=1024).step_by(16) {
                let data = root + roots / 16;
                if data >= total || (root..data).any(|n| bad.contains(&(n as u64))) {
                    continue;
                }
                for spc in [1usize, 2, 4, 8, 16, 32, 64] {
                    crate::cancellation::check()?;
                    if result.tested_layouts == budget.trials
                        || result.candidates.len() == budget.candidates
                        || work >= budget.work
                    {
                        result.search_complete_within_bounds = false;
                        break 'search;
                    }
                    result.tested_layouts += 1;
                    let clusters = (total - data) / spc;
                    if clusters == 0
                        || clusters >= 4085
                        || ((clusters + 2) * 3).div_ceil(2) > first.len()
                    {
                        continue;
                    }
                    let layout = Layout {
                        total_sectors: total,
                        sectors_per_cluster: spc,
                        reserved_sectors: reserved,
                        fat_copies: 2,
                        sectors_per_fat: spf,
                        root_entries: roots,
                        root_start: root,
                        data_start: data,
                        clusters,
                    };
                    if (2..clusters + 2).any(|c| {
                        let packed = word(first, c * 3 / 2);
                        let next = if c.is_multiple_of(2) {
                            packed & 0xfff
                        } else {
                            packed >> 4
                        };
                        next != 0 && next != 0xff7 && next < 0xff8 && !layout.valid_cluster(next)
                    }) {
                        continue;
                    }
                    if let Some(candidate) =
                        candidate(image, bad, layout, &mut work, &mut cache, budget.work)?
                    {
                        result.candidates.push(candidate);
                    }
                    if work > budget.work {
                        result.search_complete_within_bounds = false;
                        break 'search;
                    }
                }
            }
        }
    }
    // Require all bounded trials to finish; never call an early/truncated
    // search unique. Equal-ranked content-supported alternatives refuse.
    if result.search_complete_within_bounds {
        let strong = result
            .candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| c.content_anchors.len() >= 2)
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        if strong.len() == 1 {
            result.selected_candidate = Some(strong[0]);
        }
    }
    result.charged_validation_work_bytes = work;
    Ok(result)
}

fn candidate(
    image: &[u8],
    bad: &BTreeSet<u64>,
    layout: Layout,
    work: &mut u64,
    cache: &mut BTreeMap<String, Result<Option<String>, String>>,
    maximum_work: u64,
) -> Result<Option<LayoutCandidate>, String> {
    let mut anchors = vec![];
    let mut chains = 0;
    let mut owners = BTreeSet::new();
    let mut end_seen = false;
    for at in (layout.root_start * SECTOR..layout.data_start * SECTOR).step_by(32) {
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
            return Ok(None);
        }
        if entry[11] & 0x18 != 0 {
            continue;
        }
        let size = dword(entry, 28) as usize;
        if size == 0 {
            continue;
        }
        if size > image.len() {
            return Ok(None);
        }
        let chain = layout.chain(image, bad, word(entry, 26));
        if chain.error.is_some()
            || chain.clusters.len() != size.div_ceil(layout.sectors_per_cluster * SECTOR)
        {
            continue;
        }
        if chain.clusters.iter().any(|c| !owners.insert(*c)) {
            return Ok(None);
        }
        chains += 1;
        let lbas = chain
            .clusters
            .iter()
            .flat_map(|c| {
                let start = layout.cluster_start(*c);
                start..start + layout.sectors_per_cluster
            })
            .take(size.div_ceil(SECTOR))
            .map(|n| n as u64)
            .collect::<Vec<_>>();
        if lbas.iter().any(|l| bad.contains(l)) {
            continue;
        }
        *work = work.saturating_add(size as u64);
        if *work > maximum_work {
            return Ok(None);
        }
        let mut bytes = Vec::with_capacity(size);
        for lba in &lbas {
            let n = (size - bytes.len()).min(SECTOR);
            bytes.extend_from_slice(&image[*lba as usize * SECTOR..*lba as usize * SECTOR + n]);
        }
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let proof = if let Some(proof) = cache.get(&hash) {
            proof.clone()
        } else {
            let proof = crate::carving::validate_layout_anchor(&bytes, work);
            cache.insert(hash.clone(), proof.clone());
            proof
        };
        if let Ok(Some(validation)) = proof {
            anchors.push(ContentAnchor {
                directory_offset: at,
                declared_bytes: size,
                payload_sha256: hash,
                source_lbas: lbas,
                validation,
            });
        }
    }
    // A populated fixed root can have no zero end marker. That boundary
    // requires two independent content witnesses, never an opaque full region.
    if chains == 0 || !end_seen && anchors.len() < 2 {
        return Ok(None);
    }
    Ok(Some(LayoutCandidate {layout, chain_anchors:chains, content_anchors:anchors,
        confidence:"matching_readable_FAT_root_chain_shape_with_optional_structural_content_evidence_not_original_layout_proof".into(),
        original_layout_certified:false}))
}
