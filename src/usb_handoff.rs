//! Opt-in USB first-persistent-error acquisition. No device is opened here;
//! generic read/seek inputs allow exact fault and no-more-reads tests.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Summary {
    pub schema: u32,
    pub retries: usize,
    pub total_sectors: usize,
    pub attempted_sectors: usize,
    pub first_failed_lba: Option<u64>,
}
impl Summary {
    pub fn unattempted(&self) -> std::ops::Range<u64> {
        self.attempted_sectors as u64..self.total_sectors as u64
    }
}
pub(super) struct Outcome {
    pub failed: BTreeMap<usize, Vec<String>>,
    pub recovered: usize,
    pub summary: Summary,
}

pub(super) fn read(
    source: &mut (impl Read + Seek),
    output: &mut impl Write,
    geometry: DiskGeometry,
    retries: usize,
    sender: &Sender<ImagingEvent>,
    log: &mut Vec<String>,
) -> Result<Outcome, String> {
    if retries > 2
        || !geometry.looks_like_floppy()
        || geometry.total_sectors() > 8192
        || geometry.bytes_per_sector != 512
    {
        return Err("Invalid bounded USB handoff geometry/retries".into());
    }
    let total = geometry.total_sectors() as usize;
    let size = geometry.bytes_per_sector as usize;
    let spt = geometry.sectors_per_track as usize;
    let mut outcome = Outcome {
        failed: BTreeMap::new(),
        recovered: 0,
        summary: Summary {
            schema: 1,
            retries,
            total_sectors: total,
            attempted_sectors: 0,
            first_failed_lba: None,
        },
    };
    record_attempt_log(
        sender,
        log,
        format!(
            "USB_HANDOFF_POLICY | retries={retries} | first_persistent_error=stop | unattempted=unknown"
        ),
    );
    for track in 0..total / spt {
        crate::cancellation::check()?;
        let first = track * spt;
        let mut bytes = vec![0; spt * size];
        let track_result = source
            .seek(SeekFrom::Start((first * size) as u64))
            .and_then(|_| source.read_exact(&mut bytes));
        match track_result {
            Ok(()) => {
                output.write_all(&bytes).map_err(|e| e.to_string())?;
                for lba in first..first + spt {
                    crate::cancellation::check()?;
                    outcome.summary.attempted_sectors += 1;
                    let _ = sender.send(ImagingEvent::Sector {
                        lba,
                        state: SectorReadState::Good,
                    });
                }
            }
            Err(track_error) => {
                record_attempt_log(
                    sender,
                    log,
                    format!(
                        "TRACK_READ_FAILED | track={track} | fallback=sector | error={}",
                        track_error
                    ),
                );
                for lba in first..first + spt {
                    crate::cancellation::check()?;
                    let mut bytes = vec![0; size];
                    let mut errors = vec![];
                    let mut success = false;
                    for pass in 0..=retries {
                        crate::cancellation::check()?;
                        match source
                            .seek(SeekFrom::Start((lba * size) as u64))
                            .and_then(|_| source.read_exact(&mut bytes))
                        {
                            Ok(()) => {
                                success = true;
                                if pass > 0 {
                                    outcome.recovered += 1;
                                    record_attempt_log(
                                        sender,
                                        log,
                                        format!(
                                            "SECTOR_RECOVERED_AFTER_RETRY | LBA={lba} | retry_pass={pass} | direction={} | attempts_used={}",
                                            retry_direction(pass),
                                            pass + 1
                                        ),
                                    );
                                }
                                break;
                            }
                            Err(e) => {
                                errors.push(e.to_string());
                                record_attempt_log(
                                    sender,
                                    log,
                                    if pass == 0 {
                                        format!(
                                            "SECTOR_READ_FAILED | LBA={lba} | pass=initial_forward | attempt=1/{} | error={e}",
                                            retries + 1
                                        )
                                    } else {
                                        format!(
                                            "SECTOR_RETRY_FAILED | LBA={lba} | retry_pass={pass}/{retries} | direction={} | error={e}",
                                            retry_direction(pass)
                                        )
                                    },
                                );
                            }
                        }
                    }
                    outcome.summary.attempted_sectors += 1;
                    let retried = !errors.is_empty();
                    if !success {
                        bytes.fill(0);
                        outcome.summary.first_failed_lba = Some(lba as u64);
                        outcome.failed.insert(lba, errors);
                    }
                    output.write_all(&bytes).map_err(|e| e.to_string())?;
                    let _ = sender.send(ImagingEvent::Sector {
                        lba,
                        state: if !success {
                            SectorReadState::Bad
                        } else if !retried {
                            SectorReadState::Good
                        } else {
                            SectorReadState::RetryRecovered
                        },
                    });
                    if !success {
                        let start = lba + 1;
                        if start < total {
                            record_attempt_log(
                                sender,
                                log,
                                format!(
                                    "UNREAD_RANGE | first={start} | end_exclusive={total} | reason=usb_fast_handoff | zero_filled=true"
                                ),
                            );
                        }
                        // Complete the compatibility image on workstation storage.
                        // Never read the device again after the terminal failure.
                        let zero = vec![0; size];
                        for _ in start..total {
                            crate::cancellation::check()?;
                            output.write_all(&zero).map_err(|e| e.to_string())?;
                        }
                        record_attempt_log(
                            sender,
                            log,
                            format!(
                                "USB_FAST_HANDOFF | failed_lba={lba} | failed_read_sectors=1 | not_attempted={} | source_access_stopped=true | saving_evidence=true",
                                total - start
                            ),
                        );
                        return Ok(outcome);
                    }
                }
            }
        }
        let _ = sender.send(ImagingEvent::Progress {
            completed: outcome.summary.attempted_sectors,
            total,
        });
    }
    Ok(outcome)
}

#[cfg(test)]
#[path = "usb_handoff_tests.rs"]
pub(crate) mod tests;

pub(super) fn validate(
    meta: &AcquisitionMetadata,
    log: Option<&ParsedArchiverLog>,
    raw: Option<&str>,
) -> Result<(), String> {
    let Some(s) = &meta.usb_fast_handoff else {
        if log.is_some_and(|l| !l.unread_sectors.is_empty() || l.usb_handoff_retries.is_some()) {
            return Err("USB unattempted coverage lacks its handoff metadata".into());
        }
        return Ok(());
    };
    let log = log.ok_or("USB handoff lacks its acquisition log")?;
    if s.schema != 1
        || s.retries > 2
        || s.total_sectors == 0
        || s.total_sectors > 8192
        || s.attempted_sectors > s.total_sectors
        || s.first_failed_lba
            .is_some_and(|l| l >= s.total_sectors as u64)
    {
        return Err("Invalid bounded USB handoff summary".into());
    }
    let failed = s.first_failed_lba;
    let expected_attempted = failed.map_or(s.total_sectors, |l| l as usize + 1);
    let unread = s.unattempted().collect::<Vec<_>>();
    let mut expected = unread.clone();
    if let Some(l) = failed {
        expected.insert(0, l);
    }
    let actual = meta.bad_sectors.iter().map(|b| b.lba).collect::<Vec<_>>();
    if s.schema != 1
        || s.retries > 2
        || s.total_sectors == 0
        || s.total_sectors > 8192
        || s.total_sectors != meta.total_sectors
        || s.attempted_sectors != expected_attempted
        || failed.is_some_and(|l| l >= s.total_sectors as u64)
        || meta.source_backend != "windows-raw-sector"
        || meta.sector_retries != s.retries
        || meta.status != if failed.is_some() { "PARTIAL" } else { "OK" }
        || actual != expected
        || meta.bad_sector_count != expected.len()
        || log.unread_sectors != unread
        || log.bad_sectors != expected
        || log.usb_handoff_retries != Some(s.retries)
    {
        return Err(
            "USB handoff coverage/policy disagrees with sealed acquisition evidence".into(),
        );
    }
    if !log.begin_seen
        || !log.end_seen
        || log.status.label() != meta.status
        || log.disk_number != Some(meta.disk_number)
        || log.attempt_number != Some(meta.attempt_number)
        || log.geometry.total_sectors != Some(meta.total_sectors as u64)
        || log.geometry.bytes_per_sector != Some(512)
        || log.bytes_written != Some(meta.total_sectors as u64 * 512)
        || log.sha256.as_deref() != Some(meta.sha256.as_str())
    {
        return Err("USB handoff log identity/extent/hash disagrees with metadata".into());
    }
    let origins = crate::usb_passes::index(
        raw.ok_or("USB handoff raw log missing")?,
        s.total_sectors,
        &expected.iter().copied().collect(),
        meta.retry_recovered_sectors,
    )?
    .ok_or("USB handoff retry history missing")?;
    if origins.iter().any(|r| {
        r["last_attempted_pass"]
            .as_u64()
            .is_some_and(|n| n > s.retries as u64)
    }) || failed.is_some_and(|l| origins[l as usize]["last_attempted_pass"] != s.retries)
    {
        return Err("USB handoff failed-sector retry budget does not replay".into());
    }
    Ok(())
}

/// Independently replay the original USB coverage, without loading any GW
/// metadata (which would recursively re-enter confirmation verification).
pub(crate) fn require_zero_readable(metadata: &[u8], raw_log: &[u8]) -> Result<(), String> {
    let meta: AcquisitionMetadata = serde_json::from_slice(metadata).map_err(|e| e.to_string())?;
    let raw = std::str::from_utf8(raw_log).map_err(|e| e.to_string())?;
    let parsed = crate::legacy_logs::parse_archiver_log(raw)?;
    validate(&meta, Some(&parsed), Some(raw))?;
    if meta
        .usb_fast_handoff
        .as_ref()
        .is_none_or(|s| s.first_failed_lba != Some(0) || s.attempted_sectors != 1)
        || meta.bad_sectors.len() != meta.total_sectors
    {
        return Err("Custody-only confirmation requires a replay-verified first-sector USB fast handoff with zero readable bytes".into());
    }
    Ok(())
}
