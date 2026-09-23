use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use chrono::Local;
use zip::ZipArchive;

use crate::{
    conversion::{self, ConversionJob, ConversionPlanningRequest, ConversionPlanningResult},
    external_tools::{self, CommandAudit},
};

#[derive(Debug, Clone)]
pub struct ConversionRequest {
    pub planning: ConversionPlanningRequest,
    pub libreoffice_executable: PathBuf,
    pub command_audit_path: PathBuf,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone)]
pub enum ConversionEvent {
    Stage(String),
    Progress { completed: usize, total: usize },
    Finished(Result<ConversionResult, String>),
}

#[derive(Debug, Clone)]
pub struct ConversionResult {
    pub planning: ConversionPlanningResult,
    pub ok: usize,
    pub partial: usize,
    pub failed: usize,
    pub timed_out: usize,
    pub reused_outputs: usize,
    pub summary_path: PathBuf,
    pub failures_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputState {
    Ok,
    Reused,
    Failed,
    Timeout,
}

impl OutputState {
    fn label(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Reused => "REUSED",
            Self::Failed => "FAILED",
            Self::Timeout => "TIMEOUT",
        }
    }

    fn successful(self) -> bool {
        matches!(self, Self::Ok | Self::Reused)
    }
}

#[derive(Debug)]
struct OutputResult {
    state: OutputState,
    detail: String,
}

#[derive(Debug)]
struct JobResult {
    job: ConversionJob,
    modern: OutputResult,
    pdf: OutputResult,
    duration_seconds: f64,
}

impl JobResult {
    fn status(&self) -> &'static str {
        match (self.modern.state.successful(), self.pdf.state.successful()) {
            (true, true) => "OK",
            (true, false) | (false, true) => "PARTIAL",
            (false, false) => "FAILED",
        }
    }

    fn reason(&self) -> String {
        if self.status() == "OK" {
            String::new()
        } else {
            format!(
                "Modern={}; PDF={}",
                self.modern.state.label(),
                self.pdf.state.label()
            )
        }
    }
}

pub fn spawn_conversion(request: ConversionRequest) -> Receiver<ConversionEvent> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = run_conversion(
            &request,
            &|stage| {
                let _ = sender.send(ConversionEvent::Stage(stage.to_owned()));
            },
            &|completed, total| {
                let _ = sender.send(ConversionEvent::Progress { completed, total });
            },
        );
        let _ = sender.send(ConversionEvent::Finished(result));
    });
    receiver
}

fn run_conversion(
    request: &ConversionRequest,
    send_stage: &impl Fn(&str),
    send_progress: &impl Fn(usize, usize),
) -> Result<ConversionResult, String> {
    if !request.libreoffice_executable.is_file() {
        return Err(format!(
            "A LibreOffice futtatható fájl nem található: {}",
            request.libreoffice_executable.display()
        ));
    }
    if !(10..=600).contains(&request.timeout_seconds) {
        return Err("A konverziós időkorlát 10 és 600 másodperc között lehet.".to_owned());
    }

    send_stage("Delivery eredetik és friss conversion plan készítése...");
    let planning = conversion::build_conversion_plan(&request.planning, send_stage)?;
    let total = planning.jobs.len();
    let mut rows = Vec::with_capacity(total);
    for (index, job) in planning.jobs.iter().enumerate() {
        send_stage(&format!(
            "Régi Office fájl átalakítása ({}/{total}): {}",
            index + 1,
            job.original_forensic_path
        ));
        let started = Instant::now();
        let modern = convert_output(
            request,
            job,
            &job.modern_path,
            &job.modern_format.to_ascii_lowercase(),
            &job.modern_filter,
        );
        let pdf = convert_output(request, job, &job.pdf_path, "pdf", &job.pdf_filter);
        rows.push(JobResult {
            job: job.clone(),
            modern,
            pdf,
            duration_seconds: started.elapsed().as_secs_f64(),
        });
        send_progress(index + 1, total);
    }

    let summary_path = request
        .planning
        .reports_directory
        .join("ConversionSummary.csv");
    let failures_path = request
        .planning
        .reports_directory
        .join("ConversionFailures.txt");
    write_summary(&summary_path, &rows, &request.planning.converted_root)?;
    write_failures(&failures_path, &rows)?;

    Ok(ConversionResult {
        ok: rows.iter().filter(|row| row.status() == "OK").count(),
        partial: rows.iter().filter(|row| row.status() == "PARTIAL").count(),
        failed: rows.iter().filter(|row| row.status() == "FAILED").count(),
        timed_out: rows
            .iter()
            .filter(|row| {
                row.modern.state == OutputState::Timeout || row.pdf.state == OutputState::Timeout
            })
            .count(),
        reused_outputs: rows
            .iter()
            .map(|row| {
                usize::from(row.modern.state == OutputState::Reused)
                    + usize::from(row.pdf.state == OutputState::Reused)
            })
            .sum(),
        planning,
        summary_path,
        failures_path,
    })
}

fn convert_output(
    request: &ConversionRequest,
    job: &ConversionJob,
    target: &Path,
    extension: &str,
    filter: &str,
) -> OutputResult {
    match validate_output(target, extension) {
        Ok(true) => {
            return OutputResult {
                state: OutputState::Reused,
                detail: "Existing output passed integrity validation".to_owned(),
            };
        }
        Ok(false) if target.exists() => {
            return failure(format!(
                "Existing output failed integrity validation; preserved without overwrite: {}",
                target.display()
            ));
        }
        Err(error) => return failure(error),
        Ok(false) => {}
    }

    let root = std::env::temp_dir().join(format!(
        "fluxvault-office-{}-{}-{}",
        std::process::id(),
        unix_ms(),
        extension
    ));
    let output_directory = root.join("out");
    let profile_directory = root.join("profile");
    if let Err(error) = fs::create_dir(&root) {
        return failure(format!("Ideiglenes konverziós mappa hiba: {error}"));
    }
    let result = (|| {
        fs::create_dir(&output_directory)
            .map_err(|error| format!("Ideiglenes output mappa hiba: {error}"))?;
        fs::create_dir(&profile_directory)
            .map_err(|error| format!("Ideiglenes LibreOffice profil hiba: {error}"))?;
        let stdout_path = root.join("stdout.txt");
        let stderr_path = root.join("stderr.txt");
        let stdout = File::create(&stdout_path)
            .map_err(|error| format!("LibreOffice stdout fájl hiba: {error}"))?;
        let stderr = File::create(&stderr_path)
            .map_err(|error| format!("LibreOffice stderr fájl hiba: {error}"))?;
        let arguments = vec![
            "--headless".to_owned(),
            "--nologo".to_owned(),
            "--nodefault".to_owned(),
            "--norestore".to_owned(),
            "--nofirststartwizard".to_owned(),
            format!("-env:UserInstallation={}", file_uri(&profile_directory)?),
            "--convert-to".to_owned(),
            format!("{extension}:{filter}"),
            "--outdir".to_owned(),
            output_directory.display().to_string(),
            job.source_path.display().to_string(),
        ];
        let started_unix_ms = unix_ms();
        let started = Instant::now();
        let mut child = Command::new(&request.libreoffice_executable)
            .args(&arguments)
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .map_err(|error| format!("LibreOffice indítási hiba: {error}"))?;
        let mut timed_out = false;
        let exit_status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() >= Duration::from_secs(request.timeout_seconds) => {
                    timed_out = true;
                    terminate_process_tree(&mut child);
                    break child.wait().map_err(|error| {
                        format!("LibreOffice timeout utáni wait hiba: {error}")
                    })?;
                }
                Ok(None) => thread::sleep(Duration::from_millis(100)),
                Err(error) => {
                    terminate_process_tree(&mut child);
                    let _ = child.wait();
                    return Err(format!("LibreOffice wait hiba: {error}"));
                }
            }
        };
        let stdout_text = fs::read(&stdout_path)
            .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
            .unwrap_or_default();
        let stderr_text = fs::read(&stderr_path)
            .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
            .unwrap_or_default();
        let audit = CommandAudit {
            tool: "LibreOffice conversion".to_owned(),
            executable: request.libreoffice_executable.clone(),
            arguments,
            started_unix_ms,
            duration_ms: started.elapsed().as_millis(),
            success: exit_status.success() && !timed_out,
            exit_code: exit_status.code(),
            stdout: stdout_text.clone(),
            stderr: stderr_text.clone(),
        };
        external_tools::append_audit(&request.command_audit_path, &audit)?;
        if timed_out {
            return Ok(OutputResult {
                state: OutputState::Timeout,
                detail: format!(
                    "LibreOffice exceeded {} seconds; process tree stopped",
                    request.timeout_seconds
                ),
            });
        }
        if !exit_status.success() {
            return Ok(failure(format!(
                "LibreOffice exit {:?}: {} {}",
                exit_status.code(),
                stdout_text,
                stderr_text
            )));
        }
        let stem = job
            .source_path
            .file_stem()
            .ok_or_else(|| "A forrásfájlnak nincs alapneve.".to_owned())?;
        let made = output_directory.join(stem).with_extension(extension);
        if !validate_output(&made, extension)? {
            return Ok(failure(format!(
                "LibreOffice output missing or invalid: {} | {} {}",
                made.display(),
                stdout_text,
                stderr_text
            )));
        }
        if target.exists() {
            return Ok(failure(format!(
                "A cél közben létrejött; nincs felülírás: {}",
                target.display()
            )));
        }
        fs::rename(&made, target).map_err(|error| {
            format!(
                "Érvényes konverziós output előléptetési hiba {}: {error}",
                target.display()
            )
        })?;
        Ok(OutputResult {
            state: OutputState::Ok,
            detail: "Integrity validation passed".to_owned(),
        })
    })();
    let _ = fs::remove_dir_all(&root);
    match result {
        Ok(result) => result,
        Err(error) => failure(error),
    }
}

fn failure(detail: String) -> OutputResult {
    OutputResult {
        state: OutputState::Failed,
        detail,
    }
}

#[cfg(windows)]
fn terminate_process_tree(child: &mut std::process::Child) {
    let _ = Command::new("taskkill")
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
}

#[cfg(not(windows))]
fn terminate_process_tree(child: &mut std::process::Child) {
    let _ = child.kill();
}

fn file_uri(path: &Path) -> Result<String, String> {
    let absolute = path
        .canonicalize()
        .map_err(|error| format!("LibreOffice profilútvonal-hiba: {error}"))?;
    let absolute_text = absolute.to_string_lossy();
    let text = absolute_text
        .strip_prefix(r"\\?\")
        .unwrap_or(&absolute_text)
        .replace('\\', "/");
    let encoded = text
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b':' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect::<String>();
    Ok(format!("file:///{encoded}"))
}

fn validate_output(path: &Path, extension: &str) -> Result<bool, String> {
    if !path.is_file() {
        return Ok(false);
    }
    if extension.eq_ignore_ascii_case("pdf") {
        return validate_pdf(path);
    }
    let required = match extension.to_ascii_lowercase().as_str() {
        "docx" => "word/document.xml",
        "xlsx" => "xl/workbook.xml",
        "pptx" => "ppt/presentation.xml",
        _ => return Err(format!("Nem támogatott modern formátum: {extension}")),
    };
    let file = File::open(path)
        .map_err(|error| format!("OOXML fájl nem nyitható {}: {error}", path.display()))?;
    let mut archive = match ZipArchive::new(file) {
        Ok(archive) => archive,
        Err(_) => return Ok(false),
    };
    let mut has_content_types = false;
    let mut has_required = false;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format!("OOXML ZIP bejegyzés hiba: {error}"))?;
        has_content_types |= entry.name() == "[Content_Types].xml";
        has_required |= entry.name() == required;
    }
    Ok(has_content_types && has_required)
}

fn validate_pdf(path: &Path) -> Result<bool, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("PDF nem olvasható {}: {error}", path.display()))?;
    let size = file
        .metadata()
        .map_err(|error| format!("PDF metadata hiba {}: {error}", path.display()))?
        .len();
    if size < 10 {
        return Ok(false);
    }
    let mut header = [0_u8; 5];
    file.read_exact(&mut header)
        .map_err(|error| format!("PDF fejléc olvasási hiba: {error}"))?;
    if &header != b"%PDF-" {
        return Ok(false);
    }
    let tail_length = size.min(8192) as usize;
    file.seek(SeekFrom::End(-(tail_length as i64)))
        .map_err(|error| format!("PDF végének keresési hibája: {error}"))?;
    let mut tail = vec![0_u8; tail_length];
    file.read_exact(&mut tail)
        .map_err(|error| format!("PDF végének olvasási hibája: {error}"))?;
    Ok(tail.windows(5).any(|window| window == b"%%EOF"))
}

fn write_summary(path: &Path, rows: &[JobResult], converted_root: &Path) -> Result<(), String> {
    let mut file = File::create(path)
        .map_err(|error| format!("ConversionSummary nem írható {}: {error}", path.display()))?;
    file.write_all(b"\xEF\xBB\xBF")
        .map_err(|error| format!("ConversionSummary BOM hiba: {error}"))?;
    writeln!(file, "\"Floppy\",\"OriginalForensicPath\",\"DeliveryOriginalPath\",\"RecoveryMethod\",\"SourceType\",\"SourceSHA256\",\"ModernFormat\",\"ModernOK\",\"ModernResult\",\"ModernDetail\",\"ModernPath\",\"PDFOK\",\"PDFResult\",\"PDFDetail\",\"PDFPath\",\"Status\",\"PartialReason\",\"DurationSec\"")
        .map_err(|error| format!("ConversionSummary fejléc hiba: {error}"))?;
    for row in rows {
        let modern_path = if row.modern.state.successful() {
            relative_output(&row.job.modern_path, converted_root)
        } else {
            String::new()
        };
        let pdf_path = if row.pdf.state.successful() {
            relative_output(&row.job.pdf_path, converted_root)
        } else {
            String::new()
        };
        let values = [
            row.job.floppy.clone(),
            row.job.original_forensic_path.clone(),
            row.job.delivery_original_path.clone(),
            row.job.recovery_method.clone(),
            row.job.source_type.clone(),
            row.job.source_sha256.clone(),
            row.job.modern_format.clone(),
            row.modern.state.successful().to_string(),
            row.modern.state.label().to_owned(),
            row.modern.detail.clone(),
            modern_path,
            row.pdf.state.successful().to_string(),
            row.pdf.state.label().to_owned(),
            row.pdf.detail.clone(),
            pdf_path,
            row.status().to_owned(),
            row.reason(),
            format!("{:.2}", row.duration_seconds),
        ];
        writeln!(
            file,
            "{}",
            values
                .iter()
                .map(|value| format!("\"{}\"", value.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(",")
        )
        .map_err(|error| format!("ConversionSummary sorhiba: {error}"))?;
    }
    Ok(())
}

fn write_failures(path: &Path, rows: &[JobResult]) -> Result<(), String> {
    let mut text = format!(
        "\u{feff}LEGACY OFFICE CONVERSION EXCEPTIONS\r\nGenerated: {}\r\n\r\n",
        Local::now().format("%Y-%m-%d %H:%M:%S")
    );
    for row in rows.iter().filter(|row| row.status() != "OK") {
        text.push_str(&format!(
            "{} | {} | {} | {} | {} | {}\r\n",
            row.job.floppy,
            row.job.original_forensic_path,
            row.status(),
            row.reason(),
            row.modern.detail,
            row.pdf.detail
        ));
    }
    fs::write(path, text)
        .map_err(|error| format!("ConversionFailures nem írható {}: {error}", path.display()))
}

fn relative_output(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('/', "\\")
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_integrity_checks_header_and_tail() {
        let path = std::env::temp_dir().join(format!(
            "fluxvault-pdf-check-{}-{}.pdf",
            std::process::id(),
            unix_ms()
        ));
        fs::write(&path, b"%PDF-1.7\nbody\n%%EOF\n").unwrap();
        assert!(validate_pdf(&path).unwrap());
        fs::write(&path, b"%PDF-1.7\nmissing end\n").unwrap();
        assert!(!validate_pdf(&path).unwrap());
        fs::remove_file(path).unwrap();
    }

    #[test]
    #[ignore = "requires FLUXVAULT_TEST_LIBREOFFICE"]
    fn converts_a_disposable_rtf_and_reuses_valid_outputs() {
        let executable = PathBuf::from(
            std::env::var("FLUXVAULT_TEST_LIBREOFFICE")
                .expect("FLUXVAULT_TEST_LIBREOFFICE is required"),
        );
        let root = std::env::temp_dir().join(format!(
            "fluxvault-office-integration-{}-{}",
            std::process::id(),
            unix_ms()
        ));
        let source_directory = root.join("Extracted").join("001");
        fs::create_dir_all(&source_directory).unwrap();
        let source = source_directory.join("sample.rtf");
        fs::write(&source, b"{\\rtf1\\ansi FluxVault conversion test.}").unwrap();
        let request = ConversionRequest {
            planning: ConversionPlanningRequest {
                extracted_root: root.join("Extracted"),
                converted_root: root.join("Converted"),
                reports_directory: root.join("Reports"),
            },
            libreoffice_executable: executable,
            command_audit_path: root.join("Logs").join("external-tools.jsonl"),
            timeout_seconds: 45,
        };

        let first = run_conversion(&request, &|_| {}, &|_, _| {}).unwrap();
        assert_eq!(first.ok, 1);
        assert_eq!(first.reused_outputs, 0);
        assert!(first.planning.jobs[0].modern_path.is_file());
        assert!(first.planning.jobs[0].pdf_path.is_file());
        assert!(first.summary_path.is_file());

        let second = run_conversion(&request, &|_| {}, &|_, _| {}).unwrap();
        assert_eq!(second.ok, 1);
        assert_eq!(second.reused_outputs, 2);
        assert_eq!(
            fs::read(&source).unwrap(),
            b"{\\rtf1\\ansi FluxVault conversion test.}"
        );

        fs::remove_dir_all(root).unwrap();
    }
}
