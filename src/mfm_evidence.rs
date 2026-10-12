//! Independent, bounded IBM MFM evidence inspection. Never writes images or
//! chooses a winner among different CRC-valid payloads. Format references:
//! https://github.com/keirf/greaseweazle/blob/master/src/greaseweazle/codec/ibm/ibm.py
//! https://github.com/keirf/greaseweazle/blob/master/src/greaseweazle/image/scp.py
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Seek, SeekFrom},
};

const MAX_CELLS: usize = 4_000_000;
const MAX_WORDS: usize = 2_000_000;
const MAX_MARKS: usize = 1024;
const SYNC: u64 = 0x4489_4489_4489;

#[derive(Debug, Clone, Serialize)]
pub struct Observation {
    pub revolution: usize,
    pub bit_offset: usize,
    pub id: [u8; 4],
    pub id_crc_valid: bool,
    pub data_crc_valid: Option<bool>,
    pub deleted_data_mark: bool,
    pub payload_bytes: Option<usize>,
    pub payload_sha256: Option<String>,
    pub clock_violations: usize,
}
#[derive(Debug, Serialize)]
pub struct Track {
    pub cylinder: usize,
    pub head: usize,
    pub nominal_cell_ns: u32,
    pub observations: Vec<Observation>,
    pub present_ids: Vec<[u8; 4]>,
    pub valid_crc_ids: Vec<[u8; 4]>,
    pub bad_crc_observations: usize,
    pub missing_data_observations: usize,
    pub duplicate_ids_in_one_revolution: Vec<[u8; 4]>,
    pub differing_crc_valid_payload_ids: Vec<[u8; 4]>,
    pub variable_crc_observation_ids: Vec<[u8; 4]>,
    pub unusual_ids: Vec<[u8; 4]>,
    pub undecoded_intervals: usize,
    pub unknown_marks: usize,
}
#[derive(Debug, Serialize)]
pub struct Inspection {
    pub engine: &'static str,
    pub independent_of_host_sector_map: bool,
    pub physical_media_access: bool,
    pub tracks: Vec<Track>,
    pub limits: &'static str,
}
pub fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0xffffu16;
    for byte in bytes {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}
fn at(source: &mut (impl Read + Seek), offset: u64, bytes: &mut [u8]) -> Result<(), String> {
    source
        .seek(SeekFrom::Start(offset))
        .and_then(|_| source.read_exact(bytes))
        .map_err(|e| e.to_string())
}
fn le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().unwrap())
}

// Data bits are every second cell. Check ordinary MFM clocks too; A1's
// intentional missing clock is only permitted in the explicitly matched sync.
fn bytes(bits: &[u8], start: usize, count: usize) -> Option<(Vec<u8>, usize)> {
    let end = start.checked_add(count.checked_mul(16)?)?;
    let cells = bits.get(start..end)?;
    let mut previous = if start > 0 { bits[start - 1] } else { 0 };
    let mut result = Vec::with_capacity(count);
    let mut violations = 0;
    for chunk in cells.chunks_exact(16) {
        let mut value = 0;
        for pair in chunk.chunks_exact(2) {
            value = (value << 1) | pair[1];
            if pair[0] != u8::from(previous == 0 && pair[1] == 0) {
                violations += 1;
            }
            previous = pair[1];
        }
        result.push(value);
    }
    Some((result, violations))
}
fn decode(bits: &[u8], revolutions: &[u8]) -> Result<(Vec<Observation>, usize), String> {
    let mut result: Vec<Observation> = Vec::new();
    let mut pending: Option<(usize, usize)> = None;
    let (mut shift, mut position, mut unknown) = (0u64, 0usize, 0usize);
    while position < bits.len() {
        shift = ((shift << 1) | u64::from(bits[position])) & 0xffff_ffff_ffff;
        position += 1;
        if position < 48 || shift != SYNC {
            continue;
        }
        let start = position - 48;
        let Some((mark, _)) = bytes(bits, position, 1) else {
            break;
        };
        match mark[0] {
            0xfe => {
                let Some((field, violations)) = bytes(bits, position, 7) else {
                    break;
                };
                let mut crc_field = vec![0xa1; 3];
                crc_field.extend_from_slice(&field);
                let id = [field[1], field[2], field[3], field[4]];
                result.push(Observation {
                    revolution: usize::from(revolutions[start]),
                    bit_offset: start,
                    id,
                    id_crc_valid: crc16(&crc_field) == 0,
                    data_crc_valid: None,
                    deleted_data_mark: false,
                    payload_bytes: None,
                    payload_sha256: None,
                    clock_violations: violations,
                });
                if result.len() > MAX_MARKS {
                    return Err("Too many IBM MFM address marks in one track".into());
                }
                pending = Some((result.len() - 1, position + 7 * 16));
                position += 7 * 16;
                shift = 0;
            }
            0xfb | 0xf8 => {
                if let Some((index, id_end)) = pending.take()
                    && start >= id_end
                    && start - id_end <= 1000
                    && result[index].id[3] <= 7
                {
                    let size = 128usize << result[index].id[3];
                    if let Some((field, violations)) = bytes(bits, position, size + 3) {
                        let mut crc_field = vec![0xa1; 3];
                        crc_field.extend_from_slice(&field);
                        let row = &mut result[index];
                        row.data_crc_valid = Some(crc16(&crc_field) == 0);
                        row.deleted_data_mark = mark[0] == 0xf8;
                        row.payload_bytes = Some(size);
                        row.payload_sha256 =
                            Some(format!("{:x}", Sha256::digest(&field[1..size + 1])));
                        row.clock_violations += violations;
                        position += (size + 3) * 16;
                        shift = 0;
                    }
                }
            }
            _ => {
                pending = None;
                unknown += 1;
            }
        }
    }
    Ok((result, unknown))
}
fn summarize(
    cylinder: usize,
    head: usize,
    nominal: u32,
    rows: Vec<Observation>,
    intervals: usize,
    unknown: usize,
) -> Track {
    let mut ids = BTreeSet::new();
    let mut valid = BTreeSet::new();
    let mut copies = BTreeMap::<(usize, [u8; 4]), usize>::new();
    let mut payloads = BTreeMap::<[u8; 4], BTreeSet<String>>::new();
    let mut unusual = BTreeSet::new();
    let mut crc_states = BTreeMap::<[u8; 4], BTreeSet<bool>>::new();
    for row in &rows {
        ids.insert(row.id);
        *copies.entry((row.revolution, row.id)).or_default() += 1;
        if row.id_crc_valid
            && let Some(valid) = row.data_crc_valid
        {
            crc_states
                .entry(row.id)
                .or_default()
                .insert(valid && row.clock_violations == 0);
        }
        if usize::from(row.id[0]) != cylinder
            || usize::from(row.id[1]) != head
            || row.id[2] == 0
            || row.id[2] > 36
            || row.id[3] != 2
        {
            unusual.insert(row.id);
        }
        if row.id_crc_valid && row.data_crc_valid == Some(true) && row.clock_violations == 0 {
            valid.insert(row.id);
            payloads
                .entry(row.id)
                .or_default()
                .insert(row.payload_sha256.clone().unwrap());
        }
    }
    Track {
        cylinder,
        head,
        nominal_cell_ns: nominal,
        present_ids: ids.into_iter().collect(),
        valid_crc_ids: valid.into_iter().collect(),
        bad_crc_observations: rows
            .iter()
            .filter(|r| !r.id_crc_valid || r.data_crc_valid == Some(false))
            .count(),
        missing_data_observations: rows.iter().filter(|r| r.data_crc_valid.is_none()).count(),
        duplicate_ids_in_one_revolution: copies
            .into_iter()
            .filter(|(_, n)| *n > 1)
            .map(|((_, id), _)| id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        differing_crc_valid_payload_ids: payloads
            .into_iter()
            .filter(|(_, v)| v.len() > 1)
            .map(|(id, _)| id)
            .collect(),
        variable_crc_observation_ids: crc_states
            .into_iter()
            .filter(|(_, states)| states.len() > 1)
            .map(|(id, _)| id)
            .collect(),
        unusual_ids: unusual.into_iter().collect(),
        observations: rows,
        undecoded_intervals: intervals,
        unknown_marks: unknown,
    }
}

pub fn inspect(
    source: &mut (impl Read + Seek),
    size: u64,
    nominal_cell_ns: u32,
) -> Result<Inspection, String> {
    if ![1000, 2000].contains(&nominal_cell_ns) {
        return Err("IBM MFM inspection supports 250/500 kbit/s only".into());
    }
    let layout = crate::scp_diagnostics::inspect(source, size)?;
    if !layout.checksum_matches {
        return Err("Independent CRC inspection requires a matching immutable SCP checksum".into());
    }
    let mut table = [0u8; 168 * 4];
    at(source, 16, &mut table)?;
    let mut tracks = Vec::new();
    let mut observations = 0usize;
    for measured in &layout.tracks {
        if !measured.captured {
            continue;
        }
        let track_number = measured.cylinder * 2 + measured.head;
        let offset = u64::from(le(&table[track_number * 4..]));
        let nominal = f64::from(nominal_cell_ns);
        let (mut clock, mut phase, mut overflow, mut elapsed) = (nominal, 0.0, 0u64, 0.0);
        let (mut bits, mut rev_tags) = (Vec::new(), Vec::new());
        let mut periods = Vec::new();
        let mut end = 0.0;
        for r in &measured.revolutions {
            end += r.index_duration_ns as f64;
            periods.push(end);
        }
        let mut irregular = 0;
        let mut total_words = 0;
        for r in &measured.revolutions {
            let mut entry = [0u8; 12];
            at(source, offset + 4 + (r.number as u64 - 1) * 12, &mut entry)?;
            let words = le(&entry[4..]) as usize;
            total_words += words;
            if total_words > MAX_WORDS {
                return Err("SCP track exceeds independent decoder word budget".into());
            }
            let mut data = vec![0u8; words * 2];
            at(source, offset + u64::from(le(&entry[8..])), &mut data)?;
            for pair in data.chunks_exact(2) {
                let word = u64::from(u16::from_be_bytes([pair[0], pair[1]]));
                if word == 0 {
                    overflow += 65536;
                    continue;
                }
                let interval = (overflow + word) as f64 * layout.resolution_ns as f64;
                overflow = 0;
                let ticks = phase + interval;
                let cells = (ticks / clock).round().max(1.0) as usize;
                if bits.len().saturating_add(cells) > MAX_CELLS {
                    return Err("SCP track exceeds independent decoder cell budget".into());
                }
                if !(2..=4).contains(&cells) {
                    irregular += 1;
                }
                let residual = ticks - cells as f64 * clock;
                for cell in 0..cells {
                    let time = elapsed + interval * (cell + 1) as f64 / cells as f64;
                    let rev = periods.partition_point(|end| *end < time) + 1;
                    bits.push(u8::from(cell + 1 == cells));
                    rev_tags.push(rev.min(255) as u8);
                }
                elapsed += interval;
                clock += if cells <= 4 {
                    residual * 0.05
                } else {
                    (nominal - clock) * 0.05
                };
                clock = clock.clamp(nominal * 0.9, nominal * 1.1);
                phase = residual * 0.4;
            }
        }
        let (rows, unknown) = decode(&bits, &rev_tags)?;
        observations += rows.len();
        if observations > 32768 {
            return Err("SCP capture exceeds independent decoder observation budget".into());
        }
        tracks.push(summarize(
            measured.cylinder,
            measured.head,
            nominal_cell_ns,
            rows,
            irregular,
            unknown,
        ));
    }
    Ok(Inspection {
        engine: "fluxvault-ibm-mfm-crc-v1",
        independent_of_host_sector_map: true,
        physical_media_access: false,
        tracks,
        limits: "Bounded IBM MFM 250/500 kbit/s, 128..16384-byte sectors; no FM/GCR or clock-error correction. CRC-valid observations are not proof of semantic correctness or physical weakness. No flux filtering, guessed payloads, sector promotion or image mutation. Repeated revolutions are not independent captures; different valid payloads remain conflicts.",
    })
}

pub fn compare(
    inspection: &Inspection,
    image: &[u8],
    bad: &BTreeSet<u64>,
) -> Result<serde_json::Value, String> {
    use serde_json::json;
    if ![1440 * 512, 2880 * 512].contains(&image.len()) {
        return Err("Independent comparison requires a supported full image extent".into());
    }
    let spt = image.len() / 512 / 160;
    let (mut validated, mut unconfirmed, mut conflicting) = (Vec::new(), Vec::new(), Vec::new());
    let mut rows = Vec::new();
    for track in &inspection.tracks {
        let mut missing = Vec::new();
        for sector in 1..=spt {
            let lba = (track.cylinder * 2 + track.head) * spt + sector - 1;
            if lba >= image.len() / 512 {
                continue;
            }
            let id = [track.cylinder as u8, track.head as u8, sector as u8, 2];
            let observations: Vec<_> = track
                .observations
                .iter()
                .filter(|r| {
                    r.id == id
                        && r.id_crc_valid
                        && r.data_crc_valid == Some(true)
                        && r.clock_violations == 0
                        && !r.deleted_data_mark
                })
                .collect();
            let hashes: BTreeSet<_> = observations
                .iter()
                .filter_map(|r| r.payload_sha256.as_deref())
                .collect();
            if hashes.is_empty() {
                missing.push(lba);
            }
            if bad.contains(&(lba as u64)) {
                continue;
            }
            let sha = format!("{:x}", Sha256::digest(&image[lba * 512..(lba + 1) * 512]));
            let agrees = hashes.contains(sha.as_str());
            if hashes.len() > 1 || (!hashes.is_empty() && !agrees) {
                conflicting.push(lba);
            } else if agrees {
                validated.push(lba);
            } else {
                unconfirmed.push(lba);
            }
        }
        rows.push(json!({"cylinder":track.cylinder,"head":track.head,"missing_valid_expected_lbas":missing}));
    }
    Ok(
        json!({"validated_reported_good_lbas":validated,"unconfirmed_reported_good_lbas":unconfirmed,
        "conflicting_crc_valid_lbas":conflicting,"tracks":rows,
        "scope":"captured_tracks_only_uncaptured_tracks_unobserved",
        "automatic_donor_promotion":false}),
    )
}

/// Describe observed MFM IDs/sizes without guessing the missing disk extent or
/// emitting a sector image. Especially useful when fixed HD/DD trials disagree.
pub fn observe_geometry(source: &mut (impl Read + Seek), size: u64) -> serde_json::Value {
    use serde_json::json;
    let mut rates = Vec::new();
    for cell in [1000, 2000] {
        match inspect(source, size, cell) {
            Ok(i) => {
                let mut tracks = Vec::new();
                let mut total = 0usize;
                for t in &i.tracks {
                    let ids = t
                        .observations
                        .iter()
                        .filter(|r| {
                            r.id_crc_valid
                                && r.data_crc_valid == Some(true)
                                && r.clock_violations == 0
                                && !r.deleted_data_mark
                                && usize::from(r.id[0]) == t.cylinder
                                && usize::from(r.id[1]) == t.head
                        })
                        .map(|r| r.id)
                        .collect::<BTreeSet<_>>();
                    if ids.is_empty() {
                        continue;
                    }
                    total += ids.len();
                    let sectors = ids.iter().map(|id| id[2]).collect::<BTreeSet<_>>();
                    let max = sectors.iter().last().copied().unwrap_or(0);
                    let sizes = ids
                        .iter()
                        .map(|id| 128u32 << id[3])
                        .collect::<BTreeSet<_>>();
                    let contiguous =
                        max > 0 && usize::from(max) == sectors.len() && sectors.first() == Some(&1);
                    tracks.push(json!({"cylinder":t.cylinder,"head":t.head,"crc_valid_ids":ids,"observed_sector_ids":sectors,"observed_sector_sizes":sizes,
                        "observed_ids_contiguous_from_one":contiguous,"nonstandard_for_fixed_HD_DD":sizes!=BTreeSet::from([512])||!matches!(max,9|18)||!contiguous,
                        "unknown_missing_ids":true}));
                }
                rates.push(json!({"nominal_cell_ns":cell,"data_rate_kbit":if cell==1000 {500}else{250},"crc_valid_distinct_sector_observations":total,"tracks":tracks,
                    "usable_observation":total>=2,"full_geometry_established":false}));
            }
            Err(e) => {
                rates.push(json!({"nominal_cell_ns":cell,"usable_observation":false,"reason":e}))
            }
        }
    }
    let usable = rates
        .iter()
        .filter(|r| r["usable_observation"] == true)
        .count();
    json!({"engine":"fluxvault-ibm-mfm-geometry-observation-v1","rates":rates,"interpretation":match usable {0=>"no_qualified_IBM_MFM_geometry_observations",1=>"one_observed_MFM_rate_not_a_complete_format",_=>"competing_MFM_rate_observations_ambiguous"},
        "physical_media_access":false,"host_tools_invoked":false,"sector_image_created":false,"automatic_donor_eligible":false,
        "confidence":"CRC-qualified observations only; no total geometry or filesystem inferred",
        "limits":"Observed CHRN/size/rate evidence, not proof of full geometry, format family, physical label identity or file recovery. Partial/targeted/nonstandard tracks retain unknown missing IDs/extent. FM/GCR and other encodings remain unsupported queued exceptions; decoding failure does not invent a blank disk."})
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    fn encoded(bits: &mut Vec<u8>, values: &[u8]) {
        let mut previous = bits.last().copied().unwrap_or(0);
        for value in values {
            for n in (0..8).rev() {
                let data = (value >> n) & 1;
                bits.extend([u8::from(previous == 0 && data == 0), data]);
                previous = data;
            }
        }
    }
    fn sync(bits: &mut Vec<u8>) {
        encoded(bits, &[0; 12]);
        for n in (0..48).rev() {
            bits.push(((SYNC >> n) & 1) as u8);
        }
    }
    fn field(bits: &mut Vec<u8>, data: &[u8], corrupt: bool) {
        sync(bits);
        let mut bytes = vec![0xa1; 3];
        bytes.extend_from_slice(data);
        let mut crc = crc16(&bytes).to_be_bytes();
        if corrupt {
            crc[1] ^= 1;
        }
        encoded(bits, data);
        encoded(bits, &crc);
    }
    pub(crate) fn sector(
        bits: &mut Vec<u8>,
        id: [u8; 4],
        fill: u8,
        bad_id: bool,
        bad_data: bool,
        deleted: bool,
    ) {
        let mut idam = vec![0xfe];
        idam.extend(id);
        field(bits, &idam, bad_id);
        encoded(bits, &[0x4e; 22]);
        let mut payload = vec![if deleted { 0xf8 } else { 0xfb }];
        payload.extend(vec![fill; 128 << id[3]]);
        field(bits, &payload, bad_data);
        encoded(bits, &[0x4e; 30]);
    }
    pub(crate) fn fixture(bits: &[u8], cell_ns: u32, jitter: bool) -> Vec<u8> {
        let mut words = Vec::new();
        let mut pending = 0;
        for (n, bit) in bits.iter().enumerate() {
            pending += cell_ns / 25;
            if *bit == 1 {
                let ticks = pending as i64
                    + if jitter {
                        if n % 2 == 0 { 3 } else { -3 }
                    } else {
                        0
                    };
                let mut ticks = ticks as u32;
                while ticks >= 65536 {
                    words.extend(0u16.to_be_bytes());
                    ticks -= 65536;
                }
                words.extend((ticks as u16).to_be_bytes());
                pending = 0;
            }
        }
        let mut scp = vec![0; 0x2b0];
        scp[..3].copy_from_slice(b"SCP");
        scp[5] = 1;
        scp[8] = 1;
        scp[16..20].copy_from_slice(&0x2b0u32.to_le_bytes());
        scp.extend(b"TRK\0");
        scp.extend(8_000_000u32.to_le_bytes());
        scp.extend(((words.len() / 2) as u32).to_le_bytes());
        scp.extend(16u32.to_le_bytes());
        scp.extend(words);
        let sum = scp[16..]
            .iter()
            .fold(0u32, |n, b| n.wrapping_add(u32::from(*b)));
        scp[12..16].copy_from_slice(&sum.to_le_bytes());
        scp
    }
    fn parse(data: Vec<u8>, cell_ns: u32) -> Inspection {
        let size = data.len() as u64;
        inspect(&mut std::io::Cursor::new(data), size, cell_ns).unwrap()
    }
    #[test]
    fn ccitt_false_known_answer_and_residue() {
        assert_eq!(crc16(b"123456789"), 0x29b1);
        let mut data = b"123456789".to_vec();
        data.extend(0x29b1u16.to_be_bytes());
        assert_eq!(crc16(&data), 0);
    }
    #[test]
    fn clean_dd_and_hd_flux_validate_without_vendor_map() {
        for cell in [1000, 2000] {
            let mut bits = Vec::new();
            sector(&mut bits, [0, 0, 1, 2], 0x5a, false, false, false);
            let result = parse(fixture(&bits, cell, false), cell);
            let row = &result.tracks[0].observations[0];
            assert!(row.id_crc_valid);
            assert_eq!(row.data_crc_valid, Some(true));
            assert_eq!(row.clock_violations, 0);
            assert_eq!(
                row.payload_sha256,
                Some(format!("{:x}", Sha256::digest([0x5a; 512])))
            );
        }
    }
    #[test]
    fn small_timing_jitter_does_not_guess_payload() {
        let mut bits = Vec::new();
        sector(&mut bits, [0, 0, 1, 2], 0xa5, false, false, false);
        let result = parse(fixture(&bits, 2000, true), 2000);
        assert_eq!(result.tracks[0].valid_crc_ids, vec![[0, 0, 1, 2]]);
    }
    #[test]
    fn bad_id_and_bad_data_are_separate_from_missing() {
        let mut bits = Vec::new();
        sector(&mut bits, [0, 0, 1, 2], 1, true, false, false);
        sector(&mut bits, [0, 0, 2, 2], 2, false, true, false);
        let result = parse(fixture(&bits, 1000, false), 1000);
        let t = &result.tracks[0];
        assert_eq!(t.bad_crc_observations, 2);
        assert!(t.valid_crc_ids.is_empty());
        assert_eq!(t.missing_data_observations, 0);
    }
    #[test]
    fn duplicate_ids_and_different_valid_payloads_remain_conflicts() {
        let mut bits = Vec::new();
        sector(&mut bits, [0, 0, 1, 2], 1, false, false, false);
        sector(&mut bits, [0, 0, 1, 2], 2, false, false, false);
        let result = parse(fixture(&bits, 1000, false), 1000);
        let t = &result.tracks[0];
        assert_eq!(t.duplicate_ids_in_one_revolution, vec![[0, 0, 1, 2]]);
        assert_eq!(t.differing_crc_valid_payload_ids, vec![[0, 0, 1, 2]]);
        let value = compare(&result, &vec![1; 2880 * 512], &BTreeSet::new()).unwrap();
        assert_eq!(value["conflicting_crc_valid_lbas"], serde_json::json!([0]));
        assert_eq!(value["validated_reported_good_lbas"], serde_json::json!([]));
    }
    #[test]
    fn unusual_ids_and_deleted_data_marks_do_not_become_standard_donors() {
        let mut bits = Vec::new();
        sector(&mut bits, [9, 1, 42, 0], 3, false, false, true);
        let result = parse(fixture(&bits, 2000, false), 2000);
        let t = &result.tracks[0];
        assert_eq!(t.unusual_ids, vec![[9, 1, 42, 0]]);
        assert!(t.observations[0].deleted_data_mark);
        assert_eq!(t.observations[0].payload_bytes, Some(128));
        assert_eq!(
            compare(&result, &vec![3; 1440 * 512], &BTreeSet::new()).unwrap()["validated_reported_good_lbas"],
            serde_json::json!([])
        );
    }
    #[test]
    fn truncated_dam_is_missing_not_crc_valid() {
        let mut bits = Vec::new();
        sector(&mut bits, [0, 0, 1, 2], 3, false, false, false);
        bits.truncate(1000);
        let result = parse(fixture(&bits, 2000, false), 2000);
        let t = &result.tracks[0];
        assert_eq!(t.missing_data_observations, 1);
        assert!(t.valid_crc_ids.is_empty());
    }
    #[test]
    fn changed_checksum_and_invalid_rate_refuse() {
        let mut bits = Vec::new();
        sector(&mut bits, [0, 0, 1, 2], 3, false, false, false);
        let mut data = fixture(&bits, 2000, false);
        let size = data.len() as u64;
        *data.last_mut().unwrap() ^= 1;
        assert!(inspect(&mut std::io::Cursor::new(&data), size, 2000).is_err());
        assert!(inspect(&mut std::io::Cursor::new(data), size, 1234).is_err());
    }
    #[test]
    fn reported_good_bytes_must_match_crc_payload_exactly() {
        let mut bits = Vec::new();
        sector(&mut bits, [0, 0, 1, 2], 0x5a, false, false, false);
        let result = parse(fixture(&bits, 2000, false), 2000);
        let value = compare(&result, &vec![0x5a; 1440 * 512], &BTreeSet::new()).unwrap();
        assert_eq!(
            value["validated_reported_good_lbas"],
            serde_json::json!([0])
        );
        let value = compare(&result, &vec![0x5b; 1440 * 512], &BTreeSet::new()).unwrap();
        assert_eq!(value["conflicting_crc_valid_lbas"], serde_json::json!([0]));
        let value = compare(&result, &vec![0x5b; 1440 * 512], &BTreeSet::from([0])).unwrap();
        assert_eq!(value["conflicting_crc_valid_lbas"], serde_json::json!([]));
    }
    #[test]
    fn ordinary_clock_corruption_cannot_be_certified_by_data_crc_alone() {
        let mut bits = Vec::new();
        sector(&mut bits, [0, 0, 1, 2], 0x5a, false, false, false);
        // Change a clock cell, not a data cell, inside the payload.
        let mut rows = decode(&bits, &vec![1; bits.len()]).unwrap().0;
        let dam_sync = bits
            .windows(48)
            .enumerate()
            .filter(|(_, s)| s.iter().fold(0u64, |v, b| (v << 1) | u64::from(*b)) == SYNC)
            .nth(1)
            .unwrap()
            .0;
        bits[dam_sync + 64 + 32] ^= 1;
        rows.extend(decode(&bits, &vec![2; bits.len()]).unwrap().0);
        let track = summarize(0, 0, 1000, rows, 0, 0);
        assert_eq!(track.variable_crc_observation_ids, vec![[0, 0, 1, 2]]);
        let bad = &track.observations[1];
        assert_eq!(bad.data_crc_valid, Some(true));
        assert!(bad.clock_violations > 0);
    }
    #[test]
    fn repeated_revolutions_are_not_duplicate_ids_within_one_revolution() {
        let mut bits = Vec::new();
        sector(&mut bits, [0, 0, 1, 2], 0x5a, false, false, false);
        let mut rows = decode(&bits, &vec![1; bits.len()]).unwrap().0;
        rows.extend(decode(&bits, &vec![2; bits.len()]).unwrap().0);
        let track = summarize(0, 0, 1000, rows, 0, 0);
        assert!(track.duplicate_ids_in_one_revolution.is_empty());
        assert!(track.differing_crc_valid_payload_ids.is_empty());
    }
    #[test]
    fn deleted_standard_dam_is_observed_but_not_a_default_live_donor() {
        let mut bits = Vec::new();
        sector(&mut bits, [0, 0, 1, 2], 0x5a, false, false, true);
        let result = parse(fixture(&bits, 1000, false), 1000);
        assert_eq!(result.tracks[0].valid_crc_ids, vec![[0, 0, 1, 2]]);
        assert!(compare(&result,&vec![0x5a;2880*512],&BTreeSet::new()).unwrap()["validated_reported_good_lbas"].as_array().unwrap().is_empty());
    }
    #[test]
    fn impossible_flux_interval_exceeds_bound_instead_of_allocating_unlimited_cells() {
        let mut data = fixture(&[1, 0, 1], 1000, false);
        data[0x2b0 + 8..0x2b0 + 12].copy_from_slice(&3000u32.to_le_bytes());
        data.truncate(0x2b0 + 16);
        data.extend(vec![0; 5998]);
        data.extend([0, 1]);
        let sum = data[16..]
            .iter()
            .fold(0u32, |n, b| n.wrapping_add(u32::from(*b)));
        data[12..16].copy_from_slice(&sum.to_le_bytes());
        let size = data.len() as u64;
        assert!(
            inspect(&mut std::io::Cursor::new(data), size, 1000)
                .unwrap_err()
                .contains("cell budget")
        );
    }
    #[test]
    fn separated_orphan_dam_is_not_joined_to_an_unrelated_id() {
        let mut bits = Vec::new();
        field(&mut bits, &[0xfe, 0, 0, 1, 2], false);
        encoded(&mut bits, &[0x4e; 100]);
        let mut payload = vec![0xfb];
        payload.extend([0x5a; 512]);
        field(&mut bits, &payload, false);
        let rows = decode(&bits, &vec![1; bits.len()]).unwrap().0;
        assert_eq!(rows.len(), 1);
        assert!(rows[0].data_crc_valid.is_none());
    }
    #[test]
    fn nonstandard_128_byte_sector_ids_are_discovered_without_invented_image_or_extent() {
        let mut bits = Vec::new();
        for sector_id in 1..=6 {
            sector(&mut bits, [0, 0, sector_id, 0], 0x5a, false, false, false);
        }
        let scp = fixture(&bits, 2000, false);
        let size = scp.len() as u64;
        let v = observe_geometry(&mut std::io::Cursor::new(scp), size);
        assert_eq!(
            v["interpretation"],
            "one_observed_MFM_rate_not_a_complete_format"
        );
        let r = &v["rates"][1];
        assert_eq!(r["crc_valid_distinct_sector_observations"], 6);
        assert_eq!(
            r["tracks"][0]["observed_sector_sizes"],
            serde_json::json!([128])
        );
        assert_eq!(r["tracks"][0]["nonstandard_for_fixed_HD_DD"], true);
        assert_eq!(r["full_geometry_established"], false);
        assert_eq!(v["sector_image_created"], false);
    }
    #[test]
    fn no_geometry_from_false_or_bad_crc_ids_and_size_overflow_is_refused() {
        let mut bits = Vec::new();
        sector(&mut bits, [0, 0, 1, 2], 0x5a, true, false, false);
        sector(&mut bits, [0, 0, 2, 2], 0x5a, false, true, false);
        let scp = fixture(&bits, 1000, false);
        let size = scp.len() as u64;
        let v = observe_geometry(&mut std::io::Cursor::new(scp), size);
        assert_eq!(
            v["interpretation"],
            "no_qualified_IBM_MFM_geometry_observations"
        );
    }
}
