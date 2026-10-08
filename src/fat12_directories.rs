//! Corroborated allocated directory roots whose parent entry is missing.
//! Dot/self, dotdot, readable agreeing allocation and child entries are anchors,
//! not proof of historical live ownership or of the lost parent/name.
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryEvidence {
    pub start_cluster: u16,
    pub parent_cluster_hint: u16,
    pub clusters: Vec<u16>,
    pub fat_links: Vec<FatLink>,
    pub metadata_lbas: Vec<u64>,
    pub dot_entry_offset: usize,
    pub raw_anchor_hex: String,
    pub method: String,
    pub warning: String,
}

pub(super) fn discover(
    image: &[u8],
    layout: &Layout,
    bad: &BTreeSet<u64>,
    owned: &BTreeSet<u16>,
    deleted: &BTreeSet<u16>,
    known_dirs: &BTreeSet<u16>,
) -> (Vec<DirectoryEvidence>, Vec<Issue>) {
    let mut incoming = BTreeMap::<u16, usize>::new();
    let mut links = BTreeMap::new();
    for c in 2..(layout.clusters + 2) as u16 {
        if let Ok(link) = layout.link(image, bad, c) {
            if layout.valid_cluster(link.next) {
                *incoming.entry(link.next).or_default() += 1;
            }
            links.insert(c, link);
        }
    }
    let mut candidates = BTreeMap::new();
    let mut issues = Vec::new();
    for (&head, link) in &links {
        if incoming.contains_key(&head)
            || owned.contains(&head)
            || deleted.contains(&head)
            || !(layout.valid_cluster(link.next) || link.next >= 0xff8)
        {
            continue;
        }
        let at = layout.cluster_start(head) * SECTOR;
        if bad.contains(&((at / SECTOR) as u64)) {
            continue;
        }
        let dot = &image[at..at + 32];
        let parent = &image[at + 32..at + 64];
        if dot[..11] != *b".          " || parent[..11] != *b"..         " {
            continue;
        }
        let parent_cluster = word(parent, 26);
        let anchors = dot[11] == 0x10
            && parent[11] == 0x10
            && word(dot, 20) == 0
            && word(parent, 20) == 0
            && dword(dot, 28) == 0
            && dword(parent, 28) == 0
            && word(dot, 26) == head
            && parent_cluster != head
            && (parent_cluster == 0 || layout.valid_cluster(parent_cluster));
        let chain = layout.chain(image, bad, head);
        let blocked = chain.error.is_some()
            || chain.clusters.iter().any(|c| {
                owned.contains(c)
                    || deleted.contains(c)
                    || incoming.get(c).copied().unwrap_or(0) > 1
            })
            || deleted.contains(&parent_cluster)
            || (owned.contains(&parent_cluster) && !known_dirs.contains(&parent_cluster));
        if !anchors || blocked {
            issues.push(Issue { path:format!("DirectoryRecovery/cluster_{head:04}"),
                reason:"Lost-directory anchor/allocation is invalid, shared, deleted-associated or incomplete; not reconstructed".into() });
            continue;
        }
        // Require at least one plausible surviving child; do not interpret
        // arbitrary dot-like payload or empty/deleted-only allocation as a tree.
        let mut child = false;
        let mut ended = false;
        let mut readable_entries = Vec::new();
        for c in &chain.clusters {
            let begin = layout.cluster_start(*c) * SECTOR;
            for offset in (0..layout.sectors_per_cluster * SECTOR).step_by(32) {
                let offset = begin + offset;
                if offset < at + 64 && *c == head {
                    continue;
                }
                if bad.contains(&((offset / SECTOR) as u64)) {
                    continue;
                }
                let entry = &image[offset..offset + 32];
                if entry[0] == 0 {
                    ended = true;
                    break;
                }
                if entry[0] == 0xe5 || entry[11] == 0x0f || entry[11] & 8 != 0 {
                    continue;
                }
                if entry[11] & 0xc0 == 0
                    && word(entry, 20) == 0
                    && entry[..11].iter().any(|b| *b != b' ')
                    && ((entry[11] & 0x10 != 0
                        && dword(entry, 28) == 0
                        && layout.valid_cluster(word(entry, 26)))
                        || (entry[11] & 0x10 == 0
                            && dword(entry, 28) as usize <= image.len()
                            && (layout.valid_cluster(word(entry, 26))
                                || (word(entry, 26) == 0 && dword(entry, 28) == 0))))
                {
                    child = true;
                    readable_entries.push((offset / SECTOR) as u64);
                }
            }
            if ended {
                break;
            }
        }
        if !child {
            continue;
        }
        let mut metadata = chain
            .links
            .iter()
            .flat_map(|l| l.source_lbas.iter().copied())
            .collect::<BTreeSet<_>>();
        metadata.insert((at / SECTOR) as u64);
        metadata.extend(readable_entries);
        candidates.insert(head,DirectoryEvidence {
            start_cluster:head,parent_cluster_hint:parent_cluster,clusters:chain.clusters,fat_links:chain.links,
            metadata_lbas:metadata.into_iter().collect(),dot_entry_offset:at,raw_anchor_hex:hex(&image[at..at+64]),
            method:"unclaimed_allocated_directory_dot_self_parent_and_fat_anchors".into(),
            warning:"Parent path/root name and historical live/deleted ownership unknown. Known deleted allocation is excluded; reconstructed wrapper is not an original directory name. Child entries/chains are checked independently; missing entries remain unknown.".into(),
        });
        if candidates.len() > 256 {
            issues.push(Issue {
                path: "DirectoryRecovery".into(),
                reason: "Lost-directory candidate ceiling reached; no reconstructed trees exported"
                    .into(),
            });
            return (vec![], issues);
        }
    }
    // A child of another evidenced orphan is visited via that parent's actual
    // entry, not seeded twice. Missing child entries are not invented. Cycles
    // and other unrooted components are left unresolved.
    let found = candidates
        .values()
        .filter(|d| !candidates.contains_key(&d.parent_cluster_hint))
        .cloned()
        .collect();
    (found, issues)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn alloc(image: &mut [u8], c: u16, n: u16) {
        for copy in 0..2 {
            tests_support::set_fat(image, copy, c, n);
        }
    }
    use crate::fat12::tests as tests_support;
    fn directory(image: &mut [u8], c: u16, parent: u16) {
        alloc(image, c, 0xfff);
        let at = (33 + c as usize - 2) * 512;
        tests_support::entry(image, at, b".          ", c, 0, true);
        tests_support::entry(image, at + 32, b"..         ", parent, 0, true);
    }
    #[test]
    fn lost_parent_entry_recovers_nested_fragmented_file_without_inventing_root_name() {
        let mut img = tests_support::image();
        directory(&mut img, 2, 0);
        tests_support::entry(&mut img, 33 * 512 + 64, b"SUBDIR     ", 3, 0, true);
        directory(&mut img, 3, 2);
        tests_support::entry(&mut img, 34 * 512 + 64, b"SAVED   TXT", 4, 600, false);
        alloc(&mut img, 4, 9);
        alloc(&mut img, 9, 0xfff);
        img[35 * 512..36 * 512].fill(b'a');
        img[40 * 512..40 * 512 + 88].fill(b'b');
        let a = analyze(&img, &[19]).unwrap();
        assert_eq!(a.reconstructed_directories.len(), 1);
        assert_eq!(a.reconstructed_directories[0].start_cluster, 2);
        assert_eq!(a.recovered_files.len(), 1);
        let f = &a.recovered_files[0];
        assert_eq!(f.path, "DirectoryRecovery/cluster_0002/SUBDIR/SAVED.TXT");
        assert_eq!(f.clusters, vec![4, 9]);
        assert_eq!(
            file_bytes(&img, f),
            [vec![b'a'; 512], vec![b'b'; 88]].concat()
        );
        assert!(a.directory_gaps.contains(&19));
    }
    #[test]
    fn readable_normal_tree_is_not_reseeded_or_double_claimed() {
        let mut img = tests_support::image();
        directory(&mut img, 2, 0);
        tests_support::entry(&mut img, 19 * 512, b"NORMAL     ", 2, 0, true);
        tests_support::file(&mut img, 33 * 512 + 64, b"SAVED   TXT", 3, b"known");
        let a = analyze(&img, &[]).unwrap();
        assert!(a.reconstructed_directories.is_empty());
        assert_eq!(a.recovered_files[0].path, "NORMAL/SAVED.TXT");
    }
    #[test]
    fn deleted_parent_disagreement_shared_chain_and_unreadable_anchor_are_not_guessed() {
        for mode in 0..4 {
            let mut img = tests_support::image();
            directory(&mut img, 2, 0);
            tests_support::file(&mut img, 33 * 512 + 64, b"SAVED   TXT", 3, b"known");
            match mode {
                0 => {
                    tests_support::entry(&mut img, 19 * 512, b"DELETED    ", 2, 0, true);
                    img[19 * 512] = 0xe5;
                }
                1 => tests_support::set_fat(&mut img, 1, 2, 4),
                2 => {
                    alloc(&mut img, 5, 2);
                    alloc(&mut img, 6, 2);
                }
                _ => {}
            }
            let a = analyze(&img, if mode == 3 { &[33] } else { &[] }).unwrap();
            assert!(a.reconstructed_directories.is_empty(), "mode {mode}");
            assert!(a.recovered_files.is_empty());
        }
    }
    #[test]
    fn ownership_conflicts_suppress_children_of_a_reconstructed_root() {
        let mut img = tests_support::image();
        directory(&mut img, 2, 0);
        tests_support::file(&mut img, 33 * 512 + 64, b"SAVED   TXT", 3, b"known");
        tests_support::entry(&mut img, 19 * 512, b"OTHER   TXT", 3, 5, false);
        let a = analyze(&img, &[]).unwrap();
        assert!(a.recovered_files.is_empty());
        assert!(a.crosslinked_clusters.contains(&3));
    }

    #[test]
    fn fragmented_directory_crosses_a_missing_sector_without_inventing_its_entries() {
        let mut img = tests_support::image();
        directory(&mut img, 2, 0);
        alloc(&mut img, 2, 9);
        alloc(&mut img, 9, 10);
        alloc(&mut img, 10, 0xfff);
        // Occupy the first directory cluster so its zero-filled end marker
        // cannot imply an earlier end before the unreadable continuation.
        for offset in (64..512).step_by(32) {
            img[33 * SECTOR + offset] = 0xe5;
        }
        tests_support::file(&mut img, 41 * SECTOR, b"AFTER   TXT", 4, b"after hole");
        let a = analyze(&img, &[19, 40]).unwrap();
        assert_eq!(a.reconstructed_directories.len(), 1);
        assert_eq!(a.reconstructed_directories[0].clusters, vec![2, 9, 10]);
        assert_eq!(
            a.recovered_files[0].path,
            "DirectoryRecovery/cluster_0002/AFTER.TXT"
        );
        assert_eq!(file_bytes(&img, &a.recovered_files[0]), b"after hole");
        assert!(a.directory_gaps.contains(&40));
        assert!(!a.reconstructed_directories[0].metadata_lbas.contains(&40));
    }

    #[test]
    fn orphan_parent_cycles_are_not_seeded_as_certified_directory_roots() {
        let mut img = tests_support::image();
        directory(&mut img, 2, 3);
        directory(&mut img, 3, 2);
        tests_support::file(&mut img, 33 * SECTOR + 64, b"FIRST   TXT", 4, b"first");
        tests_support::file(&mut img, 34 * SECTOR + 64, b"SECOND  TXT", 5, b"second");
        let a = analyze(&img, &[]).unwrap();
        assert!(a.reconstructed_directories.is_empty());
        assert!(a.recovered_files.is_empty());
    }
}
