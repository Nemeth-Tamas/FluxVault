//! Bounded, read-only standard floppy SCP measurements. No decoder or writer.
//! Layout: https://www.cbmstuff.com/downloads/scp/scp_image_specs.txt
use serde::Serialize;
use std::io::{Read, Seek, SeekFrom};
const TABLE_END: u64 = 0x2b0;
pub(crate) const LIMIT: u64 = 512 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub(crate) struct Revolution {
    pub number: usize,
    pub index_duration_ns: u64,
    pub rpm: Option<f64>,
    pub words: u32,
    pub transitions: u64,
    pub overflow_words: u64,
    pub leading_overflow_ticks: u64,
    pub trailing_overflow_ticks: u64,
    pub shortest_interval_ns: Option<u64>,
    pub longest_interval_ns: Option<u64>,
    pub mean_interval_ns: Option<f64>,
}
#[derive(Debug, Serialize)]
pub(crate) struct Track {
    pub cylinder: usize,
    pub head: usize,
    pub captured: bool,
    pub revolutions: Vec<Revolution>,
}
#[derive(Debug, Serialize)]
pub(crate) struct Scp {
    pub resolution_ns: u64,
    pub index_aligned: bool,
    pub normalized: bool,
    pub declared_revolutions: usize,
    pub checksum_matches: bool,
    pub tracks: Vec<Track>,
}
fn u32le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().unwrap())
}
fn read_at(source: &mut (impl Read + Seek), offset: u64, buffer: &mut [u8]) -> Result<(), String> {
    source
        .seek(SeekFrom::Start(offset))
        .and_then(|_| source.read_exact(buffer))
        .map_err(|e| format!("Invalid/truncated SCP: {e}"))
}

pub(crate) fn inspect(source: &mut (impl Read + Seek), size: u64) -> Result<Scp, String> {
    if !(TABLE_END..=LIMIT).contains(&size) {
        return Err("SCP size outside bounded floppy layout".into());
    }
    let mut header = [0u8; 16];
    read_at(source, 0, &mut header)?;
    if &header[..3] != b"SCP"
        || header[5] == 0
        || header[5] > 10
        || header[6] > header[7]
        || header[7] >= 168
    {
        return Err("Unsupported SCP signature/revolution/track range".into());
    }
    if header[8] & 0x40 != 0 || header[9] != 0 || header[10] > 2 {
        return Err("Extended/non-16-bit SCP layouts are not measured".into());
    }
    // Index periods always use 25ns units. The resolution multiplier applies
    // only to flux interval words; treating both alike invents an RPM change.
    let resolution = (u64::from(header[11]) + 1) * 25;
    let revs = usize::from(header[5]);
    let mut table = [0u8; 168 * 4];
    read_at(source, 16, &mut table)?;
    let mut offsets = Vec::new();
    for track in 0..168 {
        let offset = u64::from(u32le(&table[track * 4..track * 4 + 4]));
        if offset != 0 {
            if track < usize::from(header[6])
                || track > usize::from(header[7])
                || offset < TABLE_END
                || offset + (4 + 12 * revs) as u64 > size
            {
                return Err("SCP track pointer outside declared layout".into());
            }
            if header[10] != 0 && track % 2 != usize::from(header[10] - 1) {
                return Err("Ambiguous legacy single-sided SCP track numbering".into());
            }
            offsets.push((offset, track));
        }
    }
    offsets.sort_unstable();
    if offsets.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err("Aliased SCP track headers".into());
    }
    let mut tracks = (0..168)
        .map(|n| Track {
            cylinder: n / 2,
            head: n % 2,
            captured: false,
            revolutions: vec![],
        })
        .collect::<Vec<_>>();
    let mut buffer = [0u8; 65536];
    for (position, &(offset, track)) in offsets.iter().enumerate() {
        let end = offsets.get(position + 1).map_or(size, |item| item.0);
        let header_bytes = 4 + 12 * revs;
        if offset + header_bytes as u64 > end {
            return Err("Overlapping SCP track headers".into());
        }
        let mut block = vec![0; header_bytes];
        read_at(source, offset, &mut block)?;
        if &block[..3] != b"TRK" || usize::from(block[3]) != track {
            return Err("SCP track signature/identity mismatch".into());
        }
        let mut intervals = Vec::new();
        for rev in 0..revs {
            let row = &block[4 + rev * 12..4 + (rev + 1) * 12];
            let words = u32le(&row[4..]);
            let start = offset + u64::from(u32le(&row[8..]));
            let finish = start + u64::from(words) * 2;
            if words > 0 && (start < offset + header_bytes as u64 || finish > end) {
                return Err("SCP revolution data escapes its track".into());
            }
            if words > 0 {
                intervals.push((start, finish));
            }
        }
        intervals.sort_unstable();
        if intervals.windows(2).any(|pair| pair[0].1 > pair[1].0) {
            return Err("Overlapping SCP revolutions".into());
        }
        tracks[track].captured = true;
        let mut carried_overflow = 0u64;
        for rev in 0..revs {
            let row = &block[4 + rev * 12..4 + (rev + 1) * 12];
            let duration = u64::from(u32le(row)) * 25;
            let words = u32le(&row[4..]);
            let start = offset + u64::from(u32le(&row[8..]));
            let mut remaining = u64::from(words) * 2;
            if remaining > 0 {
                source
                    .seek(SeekFrom::Start(start))
                    .map_err(|e| e.to_string())?;
            }
            let leading_overflow_ticks = carried_overflow;
            let (mut transitions, mut overflows, mut pending, mut sum) =
                (0u64, 0u64, carried_overflow, 0u64);
            let (mut min, mut max) = (None::<u64>, None::<u64>);
            while remaining > 0 {
                let n = remaining.min(buffer.len() as u64) as usize;
                source
                    .read_exact(&mut buffer[..n])
                    .map_err(|e| e.to_string())?;
                for pair in buffer[..n].chunks_exact(2) {
                    let value = u64::from(u16::from_be_bytes([pair[0], pair[1]]));
                    if value == 0 {
                        pending += 65536;
                        overflows += 1;
                        continue;
                    }
                    let ns = (pending + value) * resolution;
                    pending = 0;
                    transitions += 1;
                    sum += ns;
                    min = Some(min.map_or(ns, |v| v.min(ns)));
                    max = Some(max.map_or(ns, |v| v.max(ns)));
                }
                remaining -= n as u64;
            }
            tracks[track].revolutions.push(Revolution {
                number: rev + 1,
                index_duration_ns: duration,
                rpm: (duration > 0).then(|| 60_000_000_000f64 / duration as f64),
                words,
                transitions,
                overflow_words: overflows,
                leading_overflow_ticks,
                trailing_overflow_ticks: pending,
                shortest_interval_ns: min,
                longest_interval_ns: max,
                mean_interval_ns: (transitions > 0).then(|| sum as f64 / transitions as f64),
            });
            carried_overflow = pending;
        }
    }
    source
        .seek(SeekFrom::Start(16))
        .map_err(|e| e.to_string())?;
    let mut remaining = size - 16;
    let mut checksum = 0u32;
    while remaining > 0 {
        let n = remaining.min(buffer.len() as u64) as usize;
        source
            .read_exact(&mut buffer[..n])
            .map_err(|e| e.to_string())?;
        for byte in &buffer[..n] {
            checksum = checksum.wrapping_add(u32::from(*byte));
        }
        remaining -= n as u64;
    }
    Ok(Scp {
        resolution_ns: resolution,
        index_aligned: header[8] & 1 != 0,
        normalized: header[8] & 8 != 0,
        declared_revolutions: revs,
        checksum_matches: header[8] & 16 == 0 && checksum == u32le(&header[12..]),
        tracks,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn fixture() -> Vec<u8> {
        let mut bytes = vec![0u8; TABLE_END as usize];
        bytes[..3].copy_from_slice(b"SCP");
        bytes[5] = 2;
        bytes[7] = 1;
        bytes[8] = 1;
        bytes[16..20].copy_from_slice(&(TABLE_END as u32).to_le_bytes());
        bytes.extend_from_slice(b"TRK\0");
        for (words, offset) in [(4u32, 28u32), (2, 36)] {
            bytes.extend_from_slice(&8_000_000u32.to_le_bytes());
            bytes.extend_from_slice(&words.to_le_bytes());
            bytes.extend_from_slice(&offset.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 80, 0, 0, 0, 40, 0, 100, 0, 90, 0, 95]);
        checksum(&mut bytes);
        bytes
    }
    fn checksum(bytes: &mut [u8]) {
        let sum = bytes[16..]
            .iter()
            .fold(0u32, |n, b| n.wrapping_add(u32::from(*b)));
        bytes[12..16].copy_from_slice(&sum.to_le_bytes());
    }
    fn parse(bytes: Vec<u8>) -> Result<Scp, String> {
        let size = bytes.len() as u64;
        inspect(&mut std::io::Cursor::new(bytes), size)
    }
    #[test]
    fn periods_transitions_overflow_and_unobserved_are_not_conflated() {
        let report = parse(fixture()).unwrap();
        assert!(report.checksum_matches);
        assert!(report.tracks[0].captured);
        assert!(!report.tracks[1].captured);
        let rev = &report.tracks[0].revolutions[0];
        assert_eq!(rev.transitions, 3);
        assert_eq!(rev.overflow_words, 1);
        assert_eq!(rev.longest_interval_ns, Some((65536 + 40) * 25));
        assert_eq!(rev.rpm, Some(300.0));
        assert_eq!(report.tracks[0].revolutions[1].transitions, 2);
    }
    #[test]
    fn flux_resolution_does_not_change_index_rpm() {
        let mut data = fixture();
        data[11] = 1;
        let report = parse(data).unwrap();
        assert_eq!(report.resolution_ns, 50);
        assert_eq!(report.tracks[0].revolutions[0].rpm, Some(300.0));
        assert_eq!(
            report.tracks[0].revolutions[0].shortest_interval_ns,
            Some(80 * 50)
        );
    }
    #[test]
    fn invalid_offsets_aliases_overlaps_and_truncation_are_refused() {
        let original = fixture();
        for (offset, value) in [
            (16, 16u32),
            (20, TABLE_END as u32),
            (TABLE_END as usize + 12, 27),
            (TABLE_END as usize + 24, 30),
            (TABLE_END as usize + 8, u32::MAX),
        ] {
            let mut bytes = original.clone();
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(parse(bytes).is_err(), "{offset}");
        }
        assert!(parse(original[..original.len() - 1].to_vec()).is_err());
    }
    #[test]
    fn unsupported_layouts_and_checksum_failure_stay_explicit() {
        for (offset, value) in [(5, 11), (8, 0x40), (9, 8), (10, 2)] {
            let mut data = fixture();
            data[offset] = value;
            assert!(parse(data).is_err());
        }
        let mut data = fixture();
        *data.last_mut().unwrap() ^= 1;
        assert!(!parse(data).unwrap().checksum_matches);
    }
    #[test]
    fn trailing_overflow_is_not_a_transition() {
        let mut data = fixture();
        let n = data.len();
        data[n - 2..].copy_from_slice(&[0, 0]);
        checksum(&mut data);
        let report = parse(data).unwrap();
        assert_eq!(report.tracks[0].revolutions[1].transitions, 1);
        assert_eq!(
            report.tracks[0].revolutions[1].trailing_overflow_ticks,
            65536
        );
    }
    #[test]
    fn overflow_crossing_revolution_boundary_is_carried_not_discarded() {
        let mut data = fixture();
        data[TABLE_END as usize + 34..TABLE_END as usize + 36].fill(0);
        checksum(&mut data);
        let report = parse(data).unwrap();
        let next = &report.tracks[0].revolutions[1];
        assert_eq!(next.leading_overflow_ticks, 65536);
        assert_eq!(next.longest_interval_ns, Some((65536 + 90) * 25));
        assert_eq!(next.transitions, 2);
        assert_eq!(next.trailing_overflow_ticks, 0);
    }
}
