use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver},
    },
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
    pub workers: usize,
}

pub const DEFAULT_CONVERSION_WORKERS: usize = 4;
const CONVERSION_RETRY_DELAY: Duration = Duration::from_millis(250);
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub enum ConversionEvent {
    Stage(String),
    Progress { completed: usize, total: usize },
    Finished(Box<Result<ConversionResult, String>>),
}

#[derive(Debug, Clone)]
pub struct ConversionResult {
    pub planning: ConversionPlanningResult,
    pub ok: usize,
    pub partial: usize,
    pub failed: usize,
    pub timed_out: usize,
    pub reused_outputs: usize,
    pub retried_outputs: usize,
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
    retryable: bool,
    retry_count: u8,
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
        let _ = sender.send(ConversionEvent::Finished(Box::new(result)));
    });
    receiver
}

pub(crate) fn run_conversion(
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
    if !(1..=16).contains(&request.workers) {
        return Err("A konverziós munkaszálak száma 1 és 16 között lehet.".to_owned());
    }

    send_stage("Delivery eredetik és friss conversion plan készítése...");
    let planning = conversion::build_conversion_plan(&request.planning, send_stage)?;
    let total = planning.jobs.len();
    send_stage(&format!(
        "Régi Office fájlok átalakítása: {total} fájl, legfeljebb {} párhuzamos munkaszál...",
        request.workers
    ));
    let rows = run_bounded(
        &planning.jobs,
        request.workers,
        |job| {
            let started = Instant::now();
            let modern = retry_transient_failure(|| {
                convert_output(
                    request,
                    job,
                    &job.modern_path,
                    &job.modern_format.to_ascii_lowercase(),
                    &job.modern_filter,
                )
            });
            let pdf = retry_transient_failure(|| {
                convert_output(request, job, &job.pdf_path, "pdf", &job.pdf_filter)
            });
            JobResult {
                job: job.clone(),
                modern,
                pdf,
                duration_seconds: started.elapsed().as_secs_f64(),
            }
        },
        |completed| send_progress(completed, total),
    )?;

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
        retried_outputs: rows
            .iter()
            .map(|row| usize::from(row.modern.retry_count + row.pdf.retry_count))
            .sum(),
        planning,
        summary_path,
        failures_path,
    })
}

fn retry_transient_failure(mut attempt: impl FnMut() -> OutputResult) -> OutputResult {
    let first = attempt();
    if !first.retryable {
        return first;
    }
    thread::sleep(CONVERSION_RETRY_DELAY);
    let mut second = attempt();
    second.detail = format!("Attempt 1: {}; attempt 2: {}", first.detail, second.detail);
    second.retryable = false;
    second.retry_count = 1;
    second
}

// Results arrive in completion order, but reports must retain the plan's stable order.
fn run_bounded<T: Sync, R: Send>(
    items: &[T],
    workers: usize,
    work: impl Fn(&T) -> R + Sync,
    on_complete: impl Fn(usize),
) -> Result<Vec<R>, String> {
    let mut results: Vec<Option<R>> = std::iter::repeat_with(|| None).take(items.len()).collect();
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let next = AtomicUsize::new(0);
    thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        for _ in 0..workers.min(items.len()) {
            let sender = sender.clone();
            let next = &next;
            let work = &work;
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(index) else { break };
                    if sender.send((index, work(item))).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);
        let mut completed = 0;
        for (index, result) in receiver {
            results[index] = Some(result);
            completed += 1;
            on_complete(completed);
        }
    });
    results
        .into_iter()
        .map(|result| {
            result.ok_or_else(|| "A konverziós munkaszál eredmény nélkül leállt.".to_owned())
        })
        .collect()
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
                retryable: false,
                retry_count: 0,
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
        "fluxvault-office-{}-{}-{}-{}",
        std::process::id(),
        unix_ms(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed),
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
        let mut termination_issue = None;
        let exit_status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() >= Duration::from_secs(request.timeout_seconds) => {
                    timed_out = true;
                    termination_issue = terminate_process_tree(&mut child).err();
                    break child.wait().map_err(|error| {
                        format!("LibreOffice timeout utáni wait hiba: {error}")
                    })?;
                }
                Ok(None) => thread::sleep(Duration::from_millis(100)),
                Err(error) => {
                    let termination_issue = terminate_process_tree(&mut child).err();
                    let _ = child.wait();
                    return Err(format!(
                        "LibreOffice wait hiba: {error}{}",
                        termination_issue
                            .map(|issue| format!("; process-tree termination unverified: {issue}"))
                            .unwrap_or_default()
                    ));
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
            stderr: match &termination_issue {
                Some(issue) => {
                    format!("{stderr_text}\nProcess-tree termination unverified: {issue}")
                }
                None => stderr_text.clone(),
            },
        };
        external_tools::append_audit(&request.command_audit_path, &audit)?;
        if timed_out {
            return Ok(OutputResult {
                state: OutputState::Timeout,
                detail: match termination_issue {
                    Some(ref issue) => format!(
                        "LibreOffice exceeded {} seconds; process-tree termination unverified: {issue}",
                        request.timeout_seconds
                    ),
                    None => format!(
                        "LibreOffice exceeded {} seconds; process tree stopped",
                        request.timeout_seconds
                    ),
                },
                retryable: termination_issue.is_none(),
                retry_count: 0,
            });
        }
        if !exit_status.success() {
            return Ok(retryable_failure(format!(
                "LibreOffice exit {:?}: {} {}",
                exit_status.code(),
                stdout_text,
                stderr_text
            )));
        }
        let made = libreoffice_output_path(&output_directory, &job.source_path, extension)?;
        if !validate_output(&made, extension)? {
            return Ok(retryable_failure(format!(
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
            retryable: false,
            retry_count: 0,
        })
    })();
    let _ = fs::remove_dir_all(&root);
    match result {
        Ok(result) => result,
        Err(error) => failure(error),
    }
}

fn libreoffice_output_path(
    output_directory: &Path,
    source: &Path,
    extension: &str,
) -> Result<PathBuf, String> {
    let stem = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| "A forrásfájlnak nincs alapneve.".to_owned())?;
    // `with_extension` would treat the last dot inside "Dr. Anka" as an extension
    // and incorrectly look for "Dr.docx" instead of LibreOffice's "Dr. Anka.docx".
    Ok(output_directory.join(format!("{stem}.{extension}")))
}

fn failure(detail: String) -> OutputResult {
    OutputResult {
        state: OutputState::Failed,
        detail,
        retryable: false,
        retry_count: 0,
    }
}

fn retryable_failure(detail: String) -> OutputResult {
    OutputResult {
        state: OutputState::Failed,
        detail,
        retryable: true,
        retry_count: 0,
    }
}

#[cfg(windows)]
fn terminate_process_tree(child: &mut std::process::Child) -> Result<(), String> {
    let taskkill = std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("System32").join("taskkill.exe"))
        .ok_or_else(|| "SystemRoot is unavailable; taskkill cannot be located".to_owned());
    let outcome = taskkill.and_then(|taskkill| {
        Command::new(&taskkill)
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .output()
            .map_err(|error| format!("{} failed: {error}", taskkill.display()))
            .and_then(|output| {
                if output.status.success() {
                    Ok(())
                } else {
                    Err(format!(
                        "taskkill /T /F exited {:?}: {}",
                        output.status.code(),
                        String::from_utf8_lossy(&output.stderr).trim()
                    ))
                }
            })
    });
    // Always make a best effort to stop the direct process, even if taskkill fails.
    let _ = child.kill();
    outcome
}

#[cfg(not(windows))]
fn terminate_process_tree(child: &mut std::process::Child) -> Result<(), String> {
    let _ = child.kill();
    Err("full process-tree termination is not available on this platform".to_owned())
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

pub(crate) fn validate_output(path: &Path, extension: &str) -> Result<bool, String> {
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
    writeln!(file, "\"Floppy\",\"OriginalForensicPath\",\"DeliveryOriginalPath\",\"RecoveryMethod\",\"SourceType\",\"SourceSHA256\",\"ModernFormat\",\"ModernOK\",\"ModernResult\",\"ModernDetail\",\"ModernPath\",\"PDFOK\",\"PDFResult\",\"PDFDetail\",\"PDFPath\",\"Status\",\"PartialReason\",\"DurationSec\",\"ModernRetries\",\"PDFRetries\"")
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
            row.modern.retry_count.to_string(),
            row.pdf.retry_count.to_string(),
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
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn transient_failure_is_retried_once_and_both_attempts_are_recorded() {
        let mut calls = 0;
        let result = retry_transient_failure(|| {
            calls += 1;
            if calls == 1 {
                retryable_failure("LibreOffice exit 1".to_owned())
            } else {
                OutputResult {
                    state: OutputState::Ok,
                    detail: "Integrity validation passed".to_owned(),
                    retryable: false,
                    retry_count: 0,
                }
            }
        });
        assert_eq!(calls, 2);
        assert_eq!(result.state, OutputState::Ok);
        assert_eq!(result.retry_count, 1);
        assert!(result.detail.contains("Attempt 1: LibreOffice exit 1"));
        assert!(
            result
                .detail
                .contains("attempt 2: Integrity validation passed")
        );
    }

    #[test]
    fn unsafe_timeout_and_permanent_failure_are_not_retried() {
        for first in [
            OutputResult {
                state: OutputState::Timeout,
                detail: "process-tree termination unverified".to_owned(),
                retryable: false,
                retry_count: 0,
            },
            failure("Existing invalid output was preserved".to_owned()),
        ] {
            let mut calls = 0;
            let result = retry_transient_failure(|| {
                calls += 1;
                if calls == 1 {
                    OutputResult {
                        state: first.state,
                        detail: first.detail.clone(),
                        retryable: first.retryable,
                        retry_count: first.retry_count,
                    }
                } else {
                    panic!("a permanent failure must not be retried")
                }
            });
            assert_eq!(calls, 1);
            assert_eq!(result.state, first.state);
        }
    }

    #[test]
    fn a_second_transient_failure_stops_after_two_attempts() {
        let mut calls = 0;
        let result = retry_transient_failure(|| {
            calls += 1;
            retryable_failure(format!("LibreOffice exit {calls}"))
        });
        assert_eq!(calls, 2);
        assert_eq!(result.state, OutputState::Failed);
        assert!(!result.retryable);
        assert_eq!(result.retry_count, 1);
        assert!(result.detail.contains("Attempt 1: LibreOffice exit 1"));
        assert!(result.detail.contains("attempt 2: LibreOffice exit 2"));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires permission to terminate a disposable Windows process tree"]
    fn timeout_terminates_a_spawned_child_process_too() {
        let root = std::env::temp_dir().join(format!(
            "fluxvault-tree-test-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let parent_script = root.join("parent.ps1");
        let child_script = root.join("child.ps1");
        let ready = root.join("ready.txt");
        let child_started = root.join("child-started.txt");
        let orphan_marker = root.join("orphan.txt");
        let quote = |path: &Path| path.to_string_lossy().replace('\'', "''");
        fs::write(
            &child_script,
            format!(
                "[IO.File]::WriteAllText('{}', 'started')\nStart-Sleep -Seconds 3\n[IO.File]::WriteAllText('{}', 'orphan')\n",
                quote(&child_started),
                quote(&orphan_marker)
            ),
        )
        .unwrap();
        fs::write(
            &parent_script,
            format!(
                "$child = Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList '-NoProfile -NonInteractive -File \"{}\"' -PassThru\n[IO.File]::WriteAllText('{}', $child.Id.ToString())\nStart-Sleep -Seconds 30\n",
                quote(&child_script),
                quote(&ready)
            ),
        )
        .unwrap();
        let mut parent = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-File"])
            .arg(&parent_script)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !(ready.exists() && child_started.exists()) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }
        if !(ready.exists() && child_started.exists()) {
            let _ = terminate_process_tree(&mut parent);
            let _ = parent.wait();
            let _ = fs::remove_dir_all(&root);
            panic!("the disposable test process or its child did not start");
        }
        let child_pid = fs::read_to_string(&ready).unwrap();
        let termination = terminate_process_tree(&mut parent);
        let _ = parent.wait().unwrap();
        thread::sleep(Duration::from_millis(3500));
        let survived = orphan_marker.exists();
        if survived {
            let _ = Command::new("taskkill")
                .args(["/PID", child_pid.trim(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        fs::remove_dir_all(root).unwrap();
        assert!(termination.is_ok(), "{termination:?}");
        assert!(!survived, "a child process survived the tree kill");
    }

    #[test]
    fn bounded_workers_preserve_plan_order_and_report_completion() {
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let completed = AtomicUsize::new(0);
        let jobs: Vec<usize> = (0..12).collect();
        let results = run_bounded(
            &jobs,
            4,
            |job| {
                let running = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(running, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(if *job == 0 { 80 } else { 10 }));
                active.fetch_sub(1, Ordering::SeqCst);
                job * 2
            },
            |count| {
                assert_eq!(count, completed.fetch_add(1, Ordering::SeqCst) + 1);
            },
        )
        .unwrap();
        assert_eq!(results, jobs.iter().map(|job| job * 2).collect::<Vec<_>>());
        assert_eq!(completed.load(Ordering::SeqCst), jobs.len());
        assert!(peak.load(Ordering::SeqCst) <= 4);
        assert!(peak.load(Ordering::SeqCst) > 1);
    }

    #[test]
    fn libreoffice_output_keeps_dots_inside_source_stem() {
        assert_eq!(
            libreoffice_output_path(Path::new("out"), Path::new("Dr. Anka.doc"), "docx").unwrap(),
            PathBuf::from("out").join("Dr. Anka.docx")
        );
    }

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
        for index in 0..4 {
            fs::write(
                source_directory.join(format!("sample{index}.rtf")),
                b"{\\rtf1\\ansi FluxVault conversion test.}",
            )
            .unwrap();
        }
        let request = ConversionRequest {
            planning: ConversionPlanningRequest {
                extracted_root: root.join("Extracted"),
                converted_root: root.join("Converted"),
                reports_directory: root.join("Reports"),
            },
            libreoffice_executable: executable,
            command_audit_path: root.join("Logs").join("external-tools.jsonl"),
            timeout_seconds: 45,
            workers: DEFAULT_CONVERSION_WORKERS,
        };

        let first = run_conversion(&request, &|_| {}, &|_, _| {}).unwrap();
        assert_eq!(first.ok, 4);
        assert_eq!(first.reused_outputs, 0);
        for job in &first.planning.jobs {
            assert!(job.modern_path.is_file());
            assert!(job.pdf_path.is_file());
        }
        assert!(first.summary_path.is_file());
        let summary = fs::read_to_string(&first.summary_path).unwrap();
        assert!(summary.lines().next().unwrap().contains("ModernRetries"));
        assert!(summary.lines().next().unwrap().contains("PDFRetries"));
        let audit = fs::read_to_string(&request.command_audit_path).unwrap();
        assert_eq!(audit.lines().count(), 8);
        for line in audit.lines() {
            let record: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(record["tool"], "LibreOffice conversion");
        }

        let second = run_conversion(&request, &|_| {}, &|_, _| {}).unwrap();
        assert_eq!(second.ok, 4);
        assert_eq!(second.reused_outputs, 8);
        for index in 0..4 {
            assert_eq!(
                fs::read(source_directory.join(format!("sample{index}.rtf"))).unwrap(),
                b"{\\rtf1\\ansi FluxVault conversion test.}"
            );
        }

        fs::remove_dir_all(root).unwrap();
    }
}
