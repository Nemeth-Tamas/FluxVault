use std::{collections::BTreeMap, fs, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmdeLogStatus {
    Ok,
    Partial,
    InProgress,
}

impl DmdeLogStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Partial => "PARTIAL",
            Self::InProgress => "IN PROGRESS",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmdeDirection {
    Forward,
    Reverse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectorState {
    Copied,
    Error,
}

#[derive(Debug, Clone)]
pub struct ParsedDmdeLog {
    pub status: DmdeLogStatus,
    pub sector_size: Option<u32>,
    pub pass_count: usize,
    pub forward_passes: usize,
    pub reverse_passes: usize,
    pub copied_sectors: usize,
    pub bad_sectors: Vec<u64>,
    pub highest_sector_exclusive: u64,
    pub start_count: usize,
    pub stop_count: usize,
}

#[derive(Debug, Clone, Copy)]
struct MapRecord {
    state: SectorState,
    direction: DmdeDirection,
    start_lba: u64,
    sector_count: u64,
}

pub fn parse_dmde_log_file(path: &Path) -> Result<ParsedDmdeLog, String> {
    let content = fs::read_to_string(path).map_err(|error| {
        format!(
            "Nem sikerült beolvasni a DMDE naplót {}: {error}",
            path.display()
        )
    })?;

    parse_dmde_log(&content)
        .map_err(|error| format!("Nem felismerhető DMDE napló {}: {error}", path.display()))
}

pub fn parse_dmde_log(content: &str) -> Result<ParsedDmdeLog, String> {
    let mut sector_size = None;
    let mut start_count = 0usize;
    let mut stop_count = 0usize;
    let mut current_pass_direction = None;
    let mut pass_directions = Vec::new();
    let mut latest_states = BTreeMap::<u64, SectorState>::new();
    let mut record_count = 0usize;
    let mut highest_sector_exclusive = 0u64;

    for raw_line in content.lines() {
        let line = raw_line.trim().trim_start_matches('\u{feff}').trim();

        if line.is_empty() {
            continue;
        }

        if line.to_ascii_uppercase().starts_with("START ") {
            if let Some(direction) = current_pass_direction.take() {
                pass_directions.push(direction);
            }
            start_count += 1;
            continue;
        }

        if line.to_ascii_uppercase().starts_with("STOP ") {
            if let Some(direction) = current_pass_direction.take() {
                pass_directions.push(direction);
            }
            stop_count += 1;
            continue;
        }

        if let Some(value) = line.strip_prefix("logsec=") {
            sector_size = value.trim().parse::<u32>().ok().or(sector_size);
            continue;
        }

        let Some(record) = parse_map_record(line)? else {
            continue;
        };

        record_count += 1;

        if current_pass_direction.is_none() {
            current_pass_direction = Some(record.direction);
        }

        let end_lba = record
            .start_lba
            .checked_add(record.sector_count)
            .ok_or_else(|| "DMDE szektortartomány túlcsordulás.".to_owned())?;
        highest_sector_exclusive = highest_sector_exclusive.max(end_lba);

        for lba in record.start_lba..end_lba {
            latest_states.insert(lba, record.state);
        }
    }

    if let Some(direction) = current_pass_direction {
        pass_directions.push(direction);
    }

    if record_count == 0 {
        return Err("nem található DMDE C/E szektortérkép rekord".to_owned());
    }

    let bad_sectors = latest_states
        .iter()
        .filter_map(|(lba, state)| (*state == SectorState::Error).then_some(*lba))
        .collect::<Vec<_>>();
    let copied_sectors = latest_states
        .values()
        .filter(|state| **state == SectorState::Copied)
        .count();
    let status = if start_count > stop_count {
        DmdeLogStatus::InProgress
    } else if bad_sectors.is_empty() {
        DmdeLogStatus::Ok
    } else {
        DmdeLogStatus::Partial
    };
    let forward_passes = pass_directions
        .iter()
        .filter(|direction| **direction == DmdeDirection::Forward)
        .count();
    let reverse_passes = pass_directions
        .iter()
        .filter(|direction| **direction == DmdeDirection::Reverse)
        .count();

    Ok(ParsedDmdeLog {
        status,
        sector_size,
        pass_count: pass_directions.len().max(start_count),
        forward_passes,
        reverse_passes,
        copied_sectors,
        bad_sectors,
        highest_sector_exclusive,
        start_count,
        stop_count,
    })
}

fn parse_map_record(line: &str) -> Result<Option<MapRecord>, String> {
    let fields = line.split_whitespace().collect::<Vec<_>>();

    if fields.first().copied() != Some("C") && fields.first().copied() != Some("E") {
        return Ok(None);
    }

    if fields.len() < 7 || fields[5] != ":" {
        return Err(format!("hibás DMDE szektortérkép sor: {line}"));
    }

    let state = if fields[0] == "C" {
        SectorState::Copied
    } else {
        SectorState::Error
    };
    let direction = match (fields[2], fields[3]) {
        (">", ">") => DmdeDirection::Forward,
        ("<", "<") => DmdeDirection::Reverse,
        _ => return Err(format!("ismeretlen DMDE olvasási irány: {line}")),
    };
    let start_lba = fields[4]
        .parse::<u64>()
        .map_err(|_| format!("hibás DMDE kezdő LBA: {line}"))?;
    let sector_count = fields[6]
        .parse::<u64>()
        .map_err(|_| format!("hibás DMDE szektorszám: {line}"))?;

    if sector_count == 0 {
        return Err(format!("nulla hosszúságú DMDE szektortartomány: {line}"));
    }

    Ok(Some(MapRecord {
        state,
        direction,
        start_lba,
        sector_count,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_successful_pass_replaces_earlier_errors() {
        let parsed = parse_dmde_log(
            "START 2026-09-19 10:00:00.000\n\
             logsec=512\n\
             C 1 > > 0 : 8\n\
             E 1 > > 8 : 3\n\
             C 1 > > 11 : 9\n\
             STOP 2026-09-19 10:01:00.000\n\
             START 2026-09-19 10:02:00.000\n\
             C 1 < < 9 : 2\n\
             E 1 < < 8 : 1\n\
             STOP 2026-09-19 10:03:00.000",
        )
        .unwrap();

        assert_eq!(parsed.status, DmdeLogStatus::Partial);
        assert_eq!(parsed.sector_size, Some(512));
        assert_eq!(parsed.pass_count, 2);
        assert_eq!(parsed.forward_passes, 1);
        assert_eq!(parsed.reverse_passes, 1);
        assert_eq!(parsed.bad_sectors, vec![8]);
        assert_eq!(parsed.copied_sectors, 19);
        assert_eq!(parsed.highest_sector_exclusive, 20);
    }

    #[test]
    fn latest_sector_state_wins_even_when_it_becomes_an_error() {
        let parsed = parse_dmde_log(
            "START 2026-09-19 10:00:00.000\n\
             C 1 > > 0 : 4\n\
             STOP 2026-09-19 10:01:00.000\n\
             START 2026-09-19 10:02:00.000\n\
             E 1 < < 2 : 1\n\
             STOP 2026-09-19 10:03:00.000",
        )
        .unwrap();

        assert_eq!(parsed.bad_sectors, vec![2]);
        assert_eq!(parsed.copied_sectors, 3);
    }

    #[test]
    fn unfinished_final_pass_is_in_progress() {
        let parsed =
            parse_dmde_log("START 2026-09-19 10:00:00.000\nlogsec=512\nE 1 > > 0 : 2").unwrap();

        assert_eq!(parsed.status, DmdeLogStatus::InProgress);
        assert_eq!(parsed.start_count, 1);
        assert_eq!(parsed.stop_count, 0);
        assert_eq!(parsed.bad_sectors, vec![0, 1]);
    }
}
