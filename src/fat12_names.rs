//! Strict VFAT long-name association; damaged/unsafe names fall back to the raw alias.
//! Layout/sequence references:
//! https://github.com/torvalds/linux/blob/v6.12/include/uapi/linux/msdos_fs.h
//! https://github.com/torvalds/linux/blob/v6.12/fs/fat/dir.c
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LongNameEvidence {
    pub name: String,
    pub short_name_hex: String,
    pub alias_checksum: u8,
    pub entry_offsets: Vec<usize>,
    /// Original on-disk UTF-16 units in name order, including terminator/padding.
    pub utf16_units: Vec<u16>,
}

#[derive(Default)]
pub(crate) struct PendingName {
    slots: Vec<(usize, [u8; 32])>,
}
impl PendingName {
    pub fn clear(&mut self) {
        self.slots.clear();
    }
    pub fn push(&mut self, at: usize, entry: &[u8]) {
        if entry[0] & 0x40 != 0 {
            self.clear();
        }
        if self.slots.len() == 20 {
            self.clear();
        }
        self.slots
            .push((at, entry.try_into().expect("directory entry size")));
    }
    pub fn take(&mut self, alias: &[u8]) -> Result<Option<LongNameEvidence>, String> {
        if self.slots.is_empty() {
            return Ok(None);
        }
        let slots = std::mem::take(&mut self.slots);
        let count = slots.len();
        let checksum = checksum(alias);
        for (i, (_, slot)) in slots.iter().enumerate() {
            let ordinal = (count - i) as u8 | if i == 0 { 0x40 } else { 0 };
            if slot[0] != ordinal
                || slot[11] != 0x0f
                || slot[12] != 0
                || slot[13] != checksum
                || slot[26..28] != [0, 0]
            {
                return Err(
                    "Long-name sequence/checksum/type is invalid; short alias retained".into(),
                );
            }
        }
        let mut units = Vec::with_capacity(count * 13);
        for (_, slot) in slots.iter().rev() {
            for range in [1..11, 14..26, 28..32] {
                for at in range.step_by(2) {
                    units.push(u16::from_le_bytes([slot[at], slot[at + 1]]));
                }
            }
        }
        let length = units.iter().position(|u| *u == 0).unwrap_or(units.len());
        if length == 0
            || length > 255
            || units[..length].contains(&0xffff)
            || count != length.div_ceil(13)
            || (length < units.len() && units[length + 1..].iter().any(|u| *u != 0xffff))
        {
            return Err(
                "Long-name length/terminator/padding is invalid; short alias retained".into(),
            );
        }
        let name = String::from_utf16(&units[..length])
            .map_err(|_| "Invalid UTF-16 long name; short alias retained")?;
        if !safe_component(&name) {
            return Err("Unsafe/reserved long name; short alias retained".into());
        }
        Ok(Some(LongNameEvidence {
            name,
            short_name_hex: alias.iter().map(|b| format!("{b:02X}")).collect(),
            alias_checksum: checksum,
            entry_offsets: slots.iter().map(|(at, _)| *at).collect(),
            utf16_units: units,
        }))
    }
}
fn checksum(alias: &[u8]) -> u8 {
    alias
        .iter()
        .fold(0u8, |sum, b| sum.rotate_right(1).wrapping_add(*b))
}
fn safe_component(name: &str) -> bool {
    let upper = name.to_uppercase();
    let stem = upper
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ');
    !name.is_empty()
        && !matches!(name, "." | "..")
        && !name.ends_with([' ', '.'])
        && !name
            .chars()
            .any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))
        && !name.to_ascii_lowercase().starts_with(".fluxvault-")
        && !name.starts_with("__")
        && !["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&stem)
        && !["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|n| {
                matches!(
                    n,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub fn slots(alias: &[u8; 11], name: &str) -> Vec<[u8; 32]> {
        let mut units = name.encode_utf16().collect::<Vec<_>>();
        let count = units.len().div_ceil(13);
        if !units.len().is_multiple_of(13) {
            units.push(0);
        }
        units.resize(count * 13, 0xffff);
        (1..=count)
            .rev()
            .map(|n| {
                let mut slot = [0; 32];
                slot[0] = n as u8 | if n == count { 0x40 } else { 0 };
                slot[11] = 0x0f;
                slot[13] = checksum(alias);
                let mut chars = units[(n - 1) * 13..n * 13].iter();
                for range in [1..11, 14..26, 28..32] {
                    for at in range.step_by(2) {
                        slot[at..at + 2].copy_from_slice(&chars.next().unwrap().to_le_bytes());
                    }
                }
                slot
            })
            .collect()
    }
    fn parse(slots: &[[u8; 32]], alias: &[u8; 11]) -> Result<Option<LongNameEvidence>, String> {
        let mut pending = PendingName::default();
        for (i, slot) in slots.iter().enumerate() {
            pending.push(i * 32, slot);
        }
        pending.take(alias)
    }
    #[test]
    fn preserves_unicode_surrogates_exact_slot_names_and_padding() {
        let alias = b"DOCUME~1TXT";
        for name in [
            "Árvíztűrő tükörfúrógép.txt",
            "1234567890123",
            "123456789012😀.txt",
            "a.txt",
        ] {
            let slots = slots(alias, name);
            let recovered = parse(&slots, alias).unwrap().unwrap();
            assert_eq!(recovered.name, name);
            assert_eq!(recovered.entry_offsets.len(), slots.len());
        }
    }
    #[test]
    fn rejects_bad_checksum_sequence_type_cluster_padding_and_utf16() {
        let alias = b"DOCUME~1TXT";
        let good = slots(alias, "A moderately long filename.txt");
        for (index, offset, value) in [(0, 13, 0), (1, 0, 7), (0, 12, 1), (0, 26, 1), (0, 31, 0)] {
            let mut bad = good.clone();
            bad[index][offset] = value;
            assert!(parse(&bad, alias).is_err(), "{index}:{offset}");
        }
        let mut invalid = slots(alias, "a.txt");
        invalid[0][1..3].copy_from_slice(&0xd800u16.to_le_bytes());
        assert!(parse(&invalid, alias).is_err());
        invalid[0][1..3].copy_from_slice(&0xffffu16.to_le_bytes());
        assert!(parse(&invalid, alias).is_err());
        assert!(parse(&good[..good.len() - 1], alias).is_err());
        assert!(parse(&good[1..], alias).is_err());
        assert!(parse(&good, b"OTHERA~1TXT").is_err());
    }
    #[test]
    fn never_accepts_windows_devices_traversal_streams_or_internal_control_names() {
        let alias = b"SAFE    TXT";
        for name in [
            "..",
            "a/b",
            "a\\b",
            "a:stream",
            "CON.txt",
            "COM¹.txt",
            "LPT9",
            "CONIN$",
            "CON .txt",
            "trail.",
            "trail ",
            ".fluxvault-fat12.json",
            "__internal.txt",
            "a\n.txt",
        ] {
            assert!(parse(&slots(alias, name), alias).is_err(), "{name}");
        }
        assert!(parse(&slots(alias, &"a".repeat(255)), alias).is_ok());
        assert!(parse(&slots(alias, &"a".repeat(256)), alias).is_err());
    }
}
