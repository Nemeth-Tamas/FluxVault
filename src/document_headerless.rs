//! Narrow structural candidate when the entire CFB header is unavailable.
//! No synthetic header is constructed, and no inferred byte gets a source map.
//! MS-CFB 2.3/2.6: a single 512-byte FAT has 128 entries; stream ID 0 is
//! the first directory sector's root. The inferred geometry stays a warning.
use super::*;

pub(in crate::document_salvage) fn streams(
    bytes: &[u8],
    sources: &[Option<usize>],
) -> Result<(SparseStream, SparseStream, BTreeSet<u64>, Evidence), String> {
    if bytes.len() != sources.len()
        || !(4 * 512..=129 * 512).contains(&bytes.len())
        || !bytes.len().is_multiple_of(512)
        || sources[..512].iter().any(Option::is_some)
    {
        return Err(
            "Requires an entirely unavailable header and a bounded single-FAT 512-byte geometry"
                .into(),
        );
    }
    let sectors = bytes.len() / 512 - 1;
    let known = |at: usize, n: usize| sources[at..at + n].iter().all(Option::is_some);
    let mut fats = vec![];
    let mut roots = vec![];
    for sid in 0..sectors {
        let at = (sid + 1) * 512;
        if known(at, 512) {
            let values = (0..128)
                .map(|i| u32at(bytes, at + i * 4) as u32)
                .collect::<Vec<_>>();
            if values[sid] == FAT
                && values.iter().filter(|v| **v == FAT).count() == 1
                && values.iter().enumerate().all(|(i, v)| {
                    if i >= sectors {
                        *v == FREE
                    } else {
                        (*v as usize) < sectors || matches!(*v, END | FREE | FAT)
                    }
                })
            {
                fats.push(sid as u32);
            }
        }
        if known(at, 128)
            && let Ok(Some(e)) = parse_entry(bytes[at..at + 128].to_vec(), 0, at, 3)
            && e.kind == 5
            && e.name == "Root Entry"
            && e.left == FREE
            && e.right == FREE
        {
            roots.push(sid as u32);
        }
    }
    if fats.len() != 1 || roots.len() != 1 {
        return Err(format!(
            "Headerless anchors not unique: {} FAT / {} root candidates",
            fats.len(),
            roots.len()
        ));
    }
    let mut p = Parser {
        bytes,
        sources,
        sector: 512,
        fats: fats.clone(),
        metadata: BTreeSet::new(),
        links: BTreeMap::new(),
        visits: 0,
    };
    p.known((fats[0] as usize + 1) * 512, 512)?;
    let directory = p.walk(roots[0], None, None, 0).complete()?;
    let mut entries = vec![];
    for sid in &directory {
        let at = p.offset(*sid)?;
        for off in (0..512).step_by(128) {
            let raw = p.known(at + off, 128)?;
            if matches!(raw[66], 2 | 5) && raw[124..128].iter().any(|b| *b != 0) {
                return Err("Headerless directory has unsupported high stream-size fields".into());
            }
            entries.push(parse_entry(raw, entries.len() as u32, at + off, 3)?);
        }
    }
    let root = entries
        .first()
        .and_then(Option::as_ref)
        .ok_or("Missing inferred root")?
        .clone();
    let mut root_ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut queue = vec![root.child];
    while let Some(id) = queue.pop() {
        if id == FREE {
            continue;
        }
        if id == 0 || !root_ids.insert(id) {
            return Err("Inferred root sibling cycle/shared node".into());
        }
        let e = entries
            .get(id as usize)
            .and_then(Option::as_ref)
            .ok_or("Unavailable inferred root sibling")?;
        if e.kind == 5 || !names.insert(e.name.to_uppercase()) || (e.kind == 2 && e.child != FREE) {
            return Err("Inferred root stream identity ambiguous".into());
        }
        queue.extend([e.left, e.right]);
    }
    if entries.iter().flatten().any(|e| e.id != 0 && e.kind == 5) {
        return Err("Multiple roots in inferred directory".into());
    }
    let find = |name: &str| {
        root_ids
            .iter()
            .filter_map(|id| entries[*id as usize].as_ref())
            .find(|e| e.kind == 2 && e.name.eq_ignore_ascii_case(name))
            .cloned()
            .ok_or_else(|| format!("Inferred root lacks {name}"))
    };
    let word_entry = find("WordDocument")?;
    let mut reserved = directory.iter().copied().collect::<BTreeSet<_>>();
    reserved.insert(fats[0]);
    if root.length > 0 {
        let ids = p
            .walk(root.start, Some(root.length.div_ceil(512)), None, 0)
            .complete()?;
        for id in ids {
            if !reserved.insert(id) {
                return Err("Inferred mini-container/metadata crosslink".into());
            }
        }
    }
    // Record claims from every readable directory entry, including nested ones.
    // Mini allocation is unknown and never selected; its root container is reserved.
    let mut claims = BTreeMap::<u32, BTreeSet<u32>>::new();
    let mut walks = BTreeMap::new();
    let mut issues = vec!["Header bytes and mini-FAT identity unavailable; 512-byte single-FAT geometry is inferred, mini streams are never selected".into()];
    for e in entries
        .iter()
        .flatten()
        .filter(|e| e.kind == 2 && e.length >= 4096)
    {
        let walk = p.walk(e.start, Some(e.length.div_ceil(512)), None, 0);
        for id in &walk.ids {
            claims.entry(*id).or_default().insert(e.id);
        }
        if let Some(error) = &walk.error {
            issues.push(format!("{}: {error}", e.name));
        }
        walks.insert(e.id, walk);
    }
    if reserved.iter().any(|id| claims.contains_key(id)) {
        return Err("Inferred regular stream crosslinks metadata".into());
    }
    let mut selected = vec![];
    let mut read = |e: &Entry| -> Result<SparseStream, String> {
        if e.length < 4096 {
            return Err("Headerless mini-stream selection unsupported".into());
        }
        let ids = walks
            .remove(&e.id)
            .ok_or("Inferred selected stream not mapped")?
            .complete()?;
        if ids
            .iter()
            .any(|id| claims.get(id).is_some_and(|owners| owners.len() > 1))
        {
            return Err("Inferred selected stream has competing allocation claim".into());
        }
        let s = p.regular(&ids, e.length)?;
        selected.push(StreamEvidence {
            name: e.name.clone(),
            stream_id: e.id,
            bytes: e.length,
            mini_stream: false,
            sector_ids: ids,
        });
        Ok(s)
    };
    let word = read(&word_entry)?;
    let prefix = super::super::known(&word, 0, 14)?;
    let legacy = u16at(prefix, 0) == 0xa5dc && u16at(prefix, 2) == 0x65;
    let table = if legacy {
        SparseStream {
            bytes: vec![],
            sources: vec![],
        }
    } else {
        let table_entry = find(if u16at(prefix, 10) & 0x200 != 0 {
            "1Table"
        } else {
            "0Table"
        })?;
        read(&table_entry)?
    };
    let warning = "HEADERLESS STRUCTURAL TEXT CANDIDATE. The entire original CFB header is unavailable. A unique readable self-marked FAT and root directory corroborate assumed 512-byte single-FAT geometry; regular Word/table allocation and FIB/CLX still require validation. No header is reconstructed, mini-FAT ownership and whole-container identity/completeness remain unknown. Only mapped observed text is exported; no missing byte, formatting, object or original meaning is invented.";
    let mut root_evidence = vec![root];
    root_evidence.extend(
        root_ids
            .iter()
            .filter_map(|id| entries[*id as usize].clone()),
    );
    let evidence = Evidence {
        method: "inferred_headerless_single_FAT_regular_Word_streams".into(),
        sector_bytes: 512,
        directory_sector_ids: directory,
        missing_directory_stream_ids: vec![],
        root_sibling_entries: root_evidence
            .into_iter()
            .map(|e| EntryEvidence {
                stream_id: e.id,
                name: e.name,
                directory_parent_byte_offset: e.offset,
                raw_entry_hex: e.raw.iter().map(|b| format!("{b:02x}")).collect(),
            })
            .collect(),
        allocation_links: p.links.into_values().collect(),
        selected_streams: selected,
        allocation_issues: issues,
        ignored_storage_fields: vec![],
        inferred_header: Some(HeaderInference {
            unavailable_header_bytes: 512,
            assumed_sector_bytes: 512,
            fat_sector_id: fats[0],
            directory_first_sector_id: roots[0],
            header_reconstructed: false,
            mini_fat_mapping_known: false,
            warning: warning.into(),
        }),
        whole_container_verified: false,
        warning: warning.into(),
    };
    Ok((word, table, p.metadata, evidence))
}
