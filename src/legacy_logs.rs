use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiverLogStatus {
    Ok,
    Partial,
    Failed,
    InProgress,
    Unknown,
}

impl ArchiverLogStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Partial => "PARTIAL",
            Self::Failed => "FAILED",
            Self::InProgress => "IN PROGRESS",
            Self::Unknown => "UNKNOWN",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedGeometry {
    pub cylinders: Option<u64>,
    pub heads: Option<u32>,
    pub sectors_per_track: Option<u32>,
    pub bytes_per_sector: Option<u32>,
    pub total_sectors: Option<u64>,
    pub total_bytes: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ParsedArchiverLog {
    pub status: ArchiverLogStatus,
    pub disk_number: Option<u32>,
    pub attempt_number: Option<u32>,
    pub source: Option<String>,
    pub geometry: ParsedGeometry,
    pub bad_sectors: Vec<u64>,
    pub retry_failures: usize,
    pub retry_recovered: usize,
    pub bytes_written: Option<u64>,
    pub sha256: Option<String>,
    pub begin_seen: bool,
    pub end_seen: bool,
}

pub fn parse_archiver_log_file(path: &Path) -> Result<ParsedArchiverLog, String> {
    let content = fs::read_to_string(path).map_err(|error| {
        format!(
            "Nem sikerült beolvasni a naplót {}: {error}",
            path.display()
        )
    })?;

    parse_archiver_log(&content).map_err(|error| {
        format!(
            "Nem felismerhető acquisition napló {}: {error}",
            path.display()
        )
    })
}

pub fn parse_archiver_log(content: &str) -> Result<ParsedArchiverLog, String> {
    let mut recognized_records = 0usize;
    let mut begin_seen = false;
    let mut end_seen = false;
    let mut status = ArchiverLogStatus::Unknown;
    let mut disk_number = None;
    let mut attempt_number = None;
    let mut source = None;
    let mut geometry = ParsedGeometry::default();
    let mut bad_sectors = BTreeSet::new();
    let mut retry_failures = 0usize;
    let mut retry_recovered_events = 0usize;
    let mut retry_recovered_from_end = None;
    let mut bytes_written = None;
    let mut sha256 = None;

    for raw_line in content.lines() {
        let line = strip_timestamp(raw_line).trim();

        if line.is_empty() {
            continue;
        }

        let record_name = line
            .split(['|', ':'])
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_uppercase();
        let values = parse_key_values(line);

        match record_name.as_str() {
            "BEGIN" => {
                recognized_records += 1;
                begin_seen = true;
                disk_number = parse_u32(values.get("disk")).or(disk_number);
                attempt_number = parse_u32(values.get("attempt")).or(attempt_number);
                source = values.get("source").cloned().or(source);
            }
            "GEOMETRY" => {
                recognized_records += 1;
                geometry.cylinders = parse_u64(values.get("cylinders")).or(geometry.cylinders);
                geometry.heads = parse_u32(values.get("heads")).or(geometry.heads);
                geometry.sectors_per_track =
                    parse_u32(values.get("sectors_per_track")).or(geometry.sectors_per_track);
                geometry.bytes_per_sector =
                    parse_u32(values.get("bytes_per_sector")).or(geometry.bytes_per_sector);
                geometry.total_sectors =
                    parse_u64(values.get("total_sectors")).or(geometry.total_sectors);
                geometry.total_bytes =
                    parse_u64(values.get("total_bytes")).or(geometry.total_bytes);
            }
            "SECTOR_READ_FAILED" | "SECTOR_RETRY_FAILED" | "RETRY" => {
                recognized_records += 1;
                retry_failures += 1;
            }
            "SECTOR_RECOVERED_AFTER_RETRY" | "RETRY_RECOVERED" => {
                recognized_records += 1;
                retry_recovered_events += 1;
            }
            "BAD_SECTOR" => {
                recognized_records += 1;

                if let Some(lba) = parse_u64(values.get("lba")).or_else(|| first_integer(line)) {
                    bad_sectors.insert(lba);
                }
            }
            "SHA256" | "SHA-256" => {
                recognized_records += 1;
                sha256 = find_sha256(line).or(sha256);
            }
            "END" => {
                recognized_records += 1;
                end_seen = true;
                status = values
                    .get("status")
                    .map(|value| parse_status(value))
                    .unwrap_or(ArchiverLogStatus::Unknown);
                retry_recovered_from_end =
                    parse_usize(values.get("retry_recovered")).or(retry_recovered_from_end);
                bytes_written = parse_u64(values.get("bytes")).or(bytes_written);
                sha256 = values
                    .get("sha256")
                    .and_then(|value| normalize_sha256(value))
                    .or_else(|| find_sha256(line))
                    .or(sha256);
            }
            _ => {
                if line.to_ascii_uppercase().starts_with("SHA256=")
                    || line.to_ascii_uppercase().starts_with("SHA-256=")
                {
                    recognized_records += 1;
                    sha256 = find_sha256(line).or(sha256);
                }
            }
        }
    }

    if recognized_records == 0 {
        return Err("nem található ismert BEGIN/GEOMETRY/BAD_SECTOR/SHA256/END rekord".to_owned());
    }

    if !end_seen {
        status = ArchiverLogStatus::InProgress;
    }

    Ok(ParsedArchiverLog {
        status,
        disk_number,
        attempt_number,
        source,
        geometry,
        bad_sectors: bad_sectors.into_iter().collect(),
        retry_failures,
        retry_recovered: retry_recovered_from_end.unwrap_or(retry_recovered_events),
        bytes_written,
        sha256,
        begin_seen,
        end_seen,
    })
}

pub fn choose_primary_log<'a>(
    paths: impl IntoIterator<Item = &'a PathBuf>,
    disk_number: u32,
) -> Option<PathBuf> {
    paths
        .into_iter()
        .filter_map(|path| primary_log_score(path, disk_number).map(|score| (score, path)))
        .max_by(|(left_score, left_path), (right_score, right_path)| {
            left_score
                .cmp(right_score)
                .then_with(|| right_path.cmp(left_path))
        })
        .map(|(_, path)| path.clone())
}

fn primary_log_score(path: &Path, disk_number: u32) -> Option<u8> {
    let stem = path.file_stem()?.to_str()?.to_ascii_lowercase();
    let disk = format!("{disk_number:03}");

    if stem == disk {
        return Some(100);
    }

    if stem.starts_with(&format!("{disk}_attempt_")) {
        return Some(90);
    }

    if stem.starts_with(&disk)
        && !stem.contains("scan")
        && !stem.contains("retry")
        && !stem.contains("note")
    {
        return Some(70);
    }

    if stem.starts_with(&disk) {
        return Some(20);
    }

    None
}

fn strip_timestamp(line: &str) -> &str {
    let trimmed = line.trim();

    if trimmed.starts_with('[') {
        if let Some(end) = trimmed.find("] ") {
            return &trimmed[end + 2..];
        }
    }

    trimmed
}

fn parse_key_values(line: &str) -> BTreeMap<String, String> {
    line.split('|')
        .skip(1)
        .filter_map(|part| {
            let (key, value) = part.trim().split_once('=')?;
            Some((key.trim().to_ascii_lowercase(), value.trim().to_owned()))
        })
        .collect()
}

fn parse_status(value: &str) -> ArchiverLogStatus {
    match value.trim().to_ascii_uppercase().as_str() {
        "OK" | "SUCCESS" | "COMPLETE" => ArchiverLogStatus::Ok,
        "PARTIAL" => ArchiverLogStatus::Partial,
        "FAILED" | "FAIL" | "ERROR" => ArchiverLogStatus::Failed,
        "IN PROGRESS" | "IN_PROGRESS" | "RUNNING" => ArchiverLogStatus::InProgress,
        _ => ArchiverLogStatus::Unknown,
    }
}

fn parse_u64(value: Option<&String>) -> Option<u64> {
    value?.trim().parse().ok()
}

fn parse_u32(value: Option<&String>) -> Option<u32> {
    value?.trim().parse().ok()
}

fn parse_usize(value: Option<&String>) -> Option<usize> {
    value?.trim().parse().ok()
}

fn first_integer(line: &str) -> Option<u64> {
    line.split(|character: char| !character.is_ascii_digit())
        .find(|value| !value.is_empty())?
        .parse()
        .ok()
}

fn find_sha256(line: &str) -> Option<String> {
    line.split(|character: char| !character.is_ascii_hexdigit())
        .find_map(normalize_sha256)
}

fn normalize_sha256(value: &str) -> Option<String> {
    let value = value.trim();

    if value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit()) {
        Some(value.to_ascii_lowercase())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "f8d4b80a8863183e3e6653de7496c32362ee2c4b3e9f4ffe3ec2a17f2231878c";

    #[test]
    fn parses_completed_partial_archiver_log() {
        let log = format!(
            "[2026-09-21 21:48:30.155] BEGIN | disk=001 | attempt=002 | source=\\\\.\\A:\n\
             [2026-09-21 21:48:30.155] GEOMETRY | cylinders=80 | heads=2 | sectors_per_track=18 | bytes_per_sector=512 | total_sectors=2880 | total_bytes=1474560\n\
             [2026-09-21 21:48:36.970] SECTOR_READ_FAILED | LBA=16 | attempt=1/3\n\
             [2026-09-21 21:48:36.970] BAD_SECTOR | LBA=16 | zero_filled=true\n\
             [2026-09-21 21:49:51.502] END | status=PARTIAL | bad_sectors=1 | retry_recovered=0 | bytes=1474560 | sha256={HASH}"
        );
        let parsed = parse_archiver_log(&log).unwrap();

        assert_eq!(parsed.status, ArchiverLogStatus::Partial);
        assert_eq!(parsed.disk_number, Some(1));
        assert_eq!(parsed.attempt_number, Some(2));
        assert_eq!(parsed.geometry.total_sectors, Some(2880));
        assert_eq!(parsed.bad_sectors, vec![16]);
        assert_eq!(parsed.retry_failures, 1);
        assert_eq!(parsed.bytes_written, Some(1_474_560));
        assert_eq!(parsed.sha256.as_deref(), Some(HASH));
        assert!(parsed.begin_seen);
        assert!(parsed.end_seen);
    }

    #[test]
    fn unfinished_log_is_in_progress() {
        let parsed = parse_archiver_log(
            "BEGIN | disk=007 | source=\\\\.\\A:\nBAD_SECTOR | LBA=99 | zero_filled=true",
        )
        .unwrap();

        assert_eq!(parsed.status, ArchiverLogStatus::InProgress);
        assert!(!parsed.end_seen);
        assert_eq!(parsed.bad_sectors, vec![99]);
    }

    #[test]
    fn exact_numbered_log_outranks_auxiliary_logs() {
        let exact = PathBuf::from("C:/Logs/001.log");
        let scan = PathBuf::from("C:/Logs/001_scan.log");
        let retry = PathBuf::from("C:/Logs/001_retry_notes.log");
        let paths = [scan, retry, exact.clone()];

        assert_eq!(choose_primary_log(paths.iter(), 1), Some(exact));
    }
}
