//! Bounded format routing, not a document validator or password recovery engine.
//! Root streams only: embedded objects must not change their parent's format.
use std::{
    fs::File,
    io::{Cursor, Read},
    path::Path,
};

use serde::{Deserialize, Serialize};

const LIMIT: u64 = 8 * 1024 * 1024;
const RECORD_LIMIT: usize = 4096;
const GLOBAL_LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfficeFamily {
    Word,
    Excel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentDetection {
    pub family: Option<OfficeFamily>,
    pub format: String,
    pub evidence: String,
    pub blocked_reason: Option<String>,
}

/// Only saved regular workstation files are admitted. The finite snapshot also
/// bounds CFB allocation tables even if the original file grows during reading.
pub(crate) fn inspect(path: &Path) -> Result<Option<ContentDetection>, String> {
    crate::sector_inspection::regular(path, false)?;
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > LIMIT {
        return Ok(None);
    }
    let mut header = [0; 8];
    match file.read_exact(&mut header) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.to_string()),
    }
    if header != [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1] {
        return Ok(None);
    }
    let mut bytes = header.to_vec();
    file.take(LIMIT - 7)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMIT {
        return Ok(None);
    }
    Ok(detect(bytes))
}

fn detect(bytes: Vec<u8>) -> Option<ContentDetection> {
    let mut c = cfb::OpenOptions::new()
        .strict()
        .max_buffer_size(8192)
        .open_with(Cursor::new(bytes))
        .ok()?;
    let word = c.is_stream("/WordDocument");
    let excel = c.is_stream("/Workbook") || c.is_stream("/Book");
    let encrypted_package = c.is_stream("/EncryptionInfo") && c.is_stream("/EncryptedPackage");
    if [word, excel, encrypted_package]
        .into_iter()
        .filter(|v| *v)
        .count()
        > 1
    {
        return Some(ContentDetection {
            family: None,
            format: "ambiguous_compound".into(),
            evidence: "Multiple root application/encryption stream families".into(),
            blocked_reason: Some(
                "Ambiguous Office container; automatic conversion refused, original preserved"
                    .into(),
            ),
        });
    }
    if encrypted_package {
        return Some(ContentDetection { family: None, format: "encrypted_office_package".into(),
            evidence: "Root EncryptionInfo and EncryptedPackage streams; application unknown".into(),
            blocked_reason: Some("Password-protected Office package; original preserved, password-assisted opening required".into()) });
    }
    if word {
        let mut fib = [0; 14];
        c.open_stream("/WordDocument")
            .ok()?
            .read_exact(&mut fib)
            .ok()?;
        let ident = u16::from_le_bytes([fib[0], fib[1]]);
        let version = u16::from_le_bytes([fib[2], fib[3]]);
        let legacy = matches!(ident, 0xa5dc | 0xa5ec) && (0x65..=0x69).contains(&version);
        let modern = ident == 0xa5ec && matches!(version, 0xc1 | 0xd9 | 0x101 | 0x10c | 0x112);
        if !legacy && !modern {
            return None;
        }
        let encrypted = u16::from_le_bytes([fib[10], fib[11]]) & 0x100 != 0;
        return Some(ContentDetection { family: Some(OfficeFamily::Word),
            format: if legacy { "word_6_95" } else { "word_97_2003" }.into(),
            evidence: format!("Root WordDocument FIB: wIdent={ident:04x}, nFib={version:04x}"),
            blocked_reason: encrypted.then(|| "Password-protected Word document (FIB fEncrypted); original preserved, password-assisted opening required".into()) });
    }
    if excel {
        // Excel may carry both old/new compatibility streams. They are one
        // application, not conflicting families. Inspect both for protection;
        // prefer the newer Workbook profile when neither declares encryption.
        let paired = c.is_stream("/Book") && c.is_stream("/Workbook");
        let mut profiles = Vec::new();
        for stream in ["/Workbook", "/Book"] {
            if !c.is_stream(stream) {
                continue;
            }
            let mut book = Vec::new();
            if c.open_stream(stream)
                .ok()?
                .take((GLOBAL_LIMIT + 1) as u64)
                .read_to_end(&mut book)
                .is_err()
            {
                continue;
            }
            if let Some(profile) = excel_profile(&book, stream) {
                profiles.push(profile);
            }
        }
        let index = profiles
            .iter()
            .position(|p| p.blocked_reason.is_some())
            .unwrap_or(0);
        let mut profile = if profiles.is_empty() {
            return None;
        } else {
            profiles.remove(index)
        };
        if paired {
            profile.evidence.push_str("; both Book/Workbook compatibility streams inspected, semantic equivalence unverified");
        }
        return Some(profile);
    }
    None
}

fn excel_profile(book: &[u8], stream: &str) -> Option<ContentDetection> {
    if book.len() < 12
        || word_at(book, 0) != 0x0809
        || !(8..=16).contains(&word_at(book, 2))
        || !matches!(word_at(book, 4), 0x0500 | 0x0600)
        || word_at(book, 6) != 0x0005
    {
        return None;
    }
    let format = if word_at(book, 4) == 0x0500 {
        "excel_biff5_7"
    } else {
        "excel_biff8"
    };
    let mut offset = 0;
    for _ in 0..RECORD_LIMIT {
        if offset + 4 > book.len() || offset + 4 > GLOBAL_LIMIT {
            return Some(excel_unresolved(
                "Workbook globals truncated or detection budget exhausted",
            ));
        }
        let kind = word_at(book, offset);
        let len = usize::from(word_at(book, offset + 2));
        let end = offset + 4 + len;
        if end > book.len() || end > GLOBAL_LIMIT {
            return Some(excel_unresolved(
                "Truncated or oversized workbook global record",
            ));
        }
        if kind == 0x002f {
            if (word_at(book, 4) == 0x0500 && len != 4)
                || (word_at(book, 4) == 0x0600 && (len < 6 || word_at(book, offset + 4) > 1))
            {
                return Some(excel_unresolved(
                    "Invalid or unsupported FilePass record layout",
                ));
            }
            return Some(ContentDetection { family: Some(OfficeFamily::Excel), format: format.into(),
                    evidence: format!("Root {stream}, BOF globals, FilePass record at {offset}"),
                    blocked_reason: Some("Password-protected Excel workbook (FilePass); original preserved, password-assisted opening required".into()) });
        }
        if kind == 0x000a {
            if len != 0 {
                return Some(excel_unresolved("Invalid workbook globals EOF"));
            }
            return Some(ContentDetection {
                family: Some(OfficeFamily::Excel),
                format: format.into(),
                evidence: format!("Root {stream}, BOF globals, complete record envelope to EOF"),
                blocked_reason: None,
            });
        }
        offset = end;
    }
    Some(excel_unresolved("Workbook global record budget exhausted"))
}

fn word_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}
fn excel_unresolved(reason: &str) -> ContentDetection {
    // Inconclusive probing is not a new veto on Office's existing import path.
    // Only observed open-password protection or conflicting roots blocks a run.
    ContentDetection {
        family: Some(OfficeFamily::Excel),
        format: "excel_unresolved_globals".into(),
        evidence: reason.into(),
        blocked_reason: None,
    }
}

#[cfg(test)]
#[path = "office_content_tests.rs"]
mod tests;
