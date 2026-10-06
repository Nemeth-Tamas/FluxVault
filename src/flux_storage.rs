//! Offline lossless-compression measurements. Never replace or remove source evidence.

use crate::{flux_capture, project::ProjectState};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Cursor, Read, Write},
    time::Instant,
};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

const SAMPLE_LIMIT: u64 = 128 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct CompressionMeasurement {
    pub codec: &'static str,
    pub level: i64,
    pub source_bytes: u64,
    pub compressed_bytes: u64,
    pub saved_percent: f64,
    pub compression_seconds: f64,
    pub verification_seconds: f64,
    pub roundtrip_sha256: String,
    pub roundtrip_verified: bool,
}

#[derive(Debug, Serialize)]
pub struct StorageBenchmark {
    pub disk: u32,
    pub capture_attempt: u32,
    pub source_sha256: String,
    pub measurements: Vec<CompressionMeasurement>,
    pub source_modified: bool,
    pub physical_media_access: bool,
    pub scope: &'static str,
}

pub fn benchmark(
    project: &ProjectState,
    disk: u32,
    mut stage: impl FnMut(&str),
) -> Result<StorageBenchmark, String> {
    stage("Checking saved capture hashes; no hardware access...");
    let inspected = flux_capture::inspect_disk(project, disk)?;
    let capture = inspected
        .captures
        .iter()
        .filter(|capture| capture.status == "complete" && capture.hash_matches)
        .max_by_key(|capture| capture.bytes)
        .ok_or("No hash-verified complete raw capture available")?;
    let path = capture.raw_flux.as_ref().ok_or("Capture has no raw path")?;
    let expected = capture
        .sha256
        .as_deref()
        .ok_or("Capture has no source hash")?;
    let bytes = capture.bytes.ok_or("Capture has no recorded size")?;
    let source = crate::flux_archive::open_source(path, bytes, expected)?;
    if bytes == 0 || bytes > SAMPLE_LIMIT {
        return Err(
            "Compression benchmark is bounded to nonempty captures up to 128 MiB".to_owned(),
        );
    }
    let mut measurements = Vec::new();
    for level in [1, 6] {
        stage(&format!(
            "Measuring lossless ZIP/Deflate level {level}; checking full byte-identical decompression..."
        ));
        measurements.push(measure(
            File::open(&source.path).map_err(|e| e.to_string())?,
            bytes,
            expected,
            level,
        )?);
    }
    Ok(StorageBenchmark {
        disk,
        capture_attempt: capture.attempt,
        source_sha256: expected.to_owned(),
        measurements,
        source_modified: false,
        physical_media_access: false,
        scope: "largest_verified_capture_sample_not_entire_project",
    })
}

fn measure(
    mut input: impl Read,
    source_bytes: u64,
    expected: &str,
    level: i64,
) -> Result<CompressionMeasurement, String> {
    if source_bytes == 0 || source_bytes > SAMPLE_LIMIT {
        return Err("Invalid sample size".to_owned());
    }
    let started = Instant::now();
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            "capture.scp",
            SimpleFileOptions::default()
                .compression_method(CompressionMethod::Deflated)
                .compression_level(Some(level)),
        )
        .map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > source_bytes {
            return Err("Capture changed size during compression".to_owned());
        }
        hash.update(&buffer[..read]);
        writer
            .write_all(&buffer[..read])
            .map_err(|e| e.to_string())?;
    }
    if total != source_bytes || format!("{:x}", hash.finalize()) != expected {
        return Err("Capture changed size/hash during compression".to_owned());
    }
    let compressed = writer.finish().map_err(|e| e.to_string())?.into_inner();
    let compressed_bytes = compressed.len() as u64;
    let compression_seconds = started.elapsed().as_secs_f64();
    let verification = Instant::now();
    let mut archive = ZipArchive::new(Cursor::new(compressed)).map_err(|e| e.to_string())?;
    let mut decoded = archive.by_index(0).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut total = 0u64;
    loop {
        let read = decoded.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > source_bytes {
            return Err("Decompression exceeded recorded size".to_owned());
        }
        hash.update(&buffer[..read]);
    }
    let roundtrip_sha256 = format!("{:x}", hash.finalize());
    if total != source_bytes || roundtrip_sha256 != expected {
        return Err("Lossless roundtrip verification failed".to_owned());
    }
    Ok(CompressionMeasurement {
        codec: "zip_deflate",
        level,
        source_bytes,
        compressed_bytes,
        saved_percent: 100.0 * (1.0 - compressed_bytes as f64 / source_bytes as f64),
        compression_seconds,
        verification_seconds: verification.elapsed().as_secs_f64(),
        roundtrip_sha256,
        roundtrip_verified: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compression_is_lossless_bounded_and_refuses_changed_evidence() {
        let input = vec![42; 64 * 1024];
        let hash = format!("{:x}", Sha256::digest(&input));
        for level in [1, 6] {
            let result = measure(Cursor::new(&input), input.len() as u64, &hash, level).unwrap();
            assert!(result.roundtrip_verified);
            assert_eq!(result.roundtrip_sha256, hash);
            assert!(result.compressed_bytes < result.source_bytes);
        }
        assert!(measure(Cursor::new(&input), 1, &hash, 1).is_err());
        assert!(measure(Cursor::new(&input), input.len() as u64 + 1, &hash, 1).is_err());
        assert!(measure(Cursor::new(&input), input.len() as u64, "changed", 1).is_err());
        assert!(measure(Cursor::new(&input), SAMPLE_LIMIT + 1, &hash, 1).is_err());
        assert!(measure(Cursor::new(&input), 0, &hash, 1).is_err());
    }
}
