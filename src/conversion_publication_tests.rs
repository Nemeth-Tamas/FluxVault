use super::*;
use crate::{conversion::ConversionPlanningRequest, project::ProjectState};
struct Fixture {
    root: PathBuf,
    project: ProjectState,
    job: ConversionJob,
    made: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "fv-office-publish-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let project = ProjectState::create_without_session(root.join("project")).unwrap();
        fs::create_dir(project.extracted_dir().join("001")).unwrap();
        fs::write(
            project.extracted_dir().join("001/Notes.doc"),
            b"{\\rtf1 Original fixture bytes}",
        )
        .unwrap();
        let job = crate::conversion::build_conversion_plan(
            &ConversionPlanningRequest {
                extracted_root: project.extracted_dir(),
                converted_root: project.converted_dir(),
                reports_directory: project.reports_dir(),
            },
            &|_| {},
        )
        .unwrap()
        .jobs
        .remove(0);
        let made = root.join("made.docx");
        let mut z = zip::ZipWriter::new(File::create(&made).unwrap());
        for (name, data) in [
            ("[Content_Types].xml", b"<Types/>".as_slice()),
            (
                "word/document.xml",
                b"<document>fixture</document>".as_slice(),
            ),
        ] {
            z.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap().sync_all().unwrap();
        Self {
            root,
            project,
            job,
            made,
        }
    }
    fn context(&self) -> Context {
        context(
            &self.project.reports_dir(),
            &self.job,
            &self.job.modern_path,
            "docx",
            &self.job.modern_filter,
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn sealed_and_published_outputs_resume_without_aggregate_conversion_state_or_host() {
    for point in ["sealed", "published"] {
        let f = Fixture::new();
        let expected = bounded_hash(&f.made).unwrap();
        let result = f.context().publish(&f.made, &|stage| {
            if stage == point {
                Err("simulated process exit".into())
            } else {
                Ok(())
            }
        });
        assert!(result.is_err());
        assert!(
            !f.project
                .reports_dir()
                .join("ConversionState.json")
                .exists()
        );
        assert_eq!(f.context().replay().unwrap(), Some(expected.clone()));
        assert_eq!(bounded_hash(&f.job.modern_path).unwrap(), expected);
        assert_eq!(f.context().replay().unwrap(), Some(expected));
    }
}
#[test]
fn unsealed_scratch_is_not_adopted_and_later_publication_preserves_it() {
    let f = Fixture::new();
    let c = f.context();
    assert!(
        c.publish(&f.made, &|s| if s == "payload" {
            Err("exit".into())
        } else {
            Ok(())
        })
        .is_err()
    );
    assert_eq!(c.replay().unwrap(), None);
    let first = fs::read_dir(&c.directory)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("payload");
    let original = fs::read(&first).unwrap();
    c.publish(&f.made, &|_| Ok(())).unwrap();
    assert_eq!(fs::read(first).unwrap(), original);
    assert_eq!(c.replay().unwrap(), Some(hash(&original)));
}
#[test]
fn private_input_is_exact_and_source_mutation_refuses_prepare_and_replay() {
    let f = Fixture::new();
    let c = f.context();
    let private = f.root.join("private.doc");
    c.snapshot_source(&private).unwrap();
    assert_eq!(
        fs::read(&private).unwrap(),
        fs::read(&f.job.source_path).unwrap()
    );
    fs::write(&f.job.source_path, b"changed original").unwrap();
    assert!(c.publish(&f.made, &|_| Ok(())).is_err());
    assert!(c.replay().is_err());
    assert!(!f.job.modern_path.exists());
}
#[test]
fn changed_receipts_payloads_outputs_and_settings_are_never_silently_reused() {
    for case in ["receipt", "payload", "target", "settings"] {
        let f = Fixture::new();
        let c = f.context();
        c.publish(&f.made, &|_| Ok(())).unwrap();
        let dir = fs::read_dir(&c.directory)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        match case {
            "receipt" => fs::write(dir.join("receipt.json"), b"{}").unwrap(),
            "payload" => fs::write(dir.join("payload"), b"edited").unwrap(),
            "target" => fs::write(&f.job.modern_path, b"operator edit").unwrap(),
            _ => {}
        }
        if case == "settings" {
            let other = context(
                &f.project.reports_dir(),
                &f.job,
                &f.job.modern_path,
                "docx",
                "different filter",
            )
            .unwrap();
            assert_eq!(other.replay().unwrap(), None); // No authority to adopt the existing file.
        } else {
            assert!(c.replay().is_err(), "{case}");
        }
    }
}
#[test]
fn late_destination_collision_preserves_operator_output_and_sealed_payload() {
    let f = Fixture::new();
    let c = f.context();
    assert!(
        c.publish(&f.made, &|stage| {
            if stage == "sealed" {
                fs::write(&f.job.modern_path, b"operator file").unwrap();
            }
            Ok(())
        })
        .is_err()
    );
    assert_eq!(fs::read(&f.job.modern_path).unwrap(), b"operator file");
    assert!(c.replay().is_err());
}
#[test]
fn prepared_receipts_do_not_authorize_cross_project_or_escaped_outputs() {
    let f = Fixture::new();
    assert!(
        context(
            &f.project.reports_dir(),
            &f.job,
            &f.root.join("outside.docx"),
            "docx",
            &f.job.modern_filter
        )
        .is_err()
    );
    assert!(
        context(
            &f.project.reports_dir(),
            &f.job,
            &f.job.modern_path,
            "exe",
            &f.job.modern_filter
        )
        .is_err()
    );
    let mut wrong = f.job.clone();
    wrong.source_path = f.made.clone();
    assert!(
        context(
            &f.project.reports_dir(),
            &wrong,
            &f.job.modern_path,
            "docx",
            &f.job.modern_filter
        )
        .is_err()
    );
}

#[test]
fn published_file_is_an_independent_copy_and_missing_target_replays_the_sealed_bytes() {
    let f = Fixture::new();
    let c = f.context();
    let sha = c.publish(&f.made, &|_| Ok(())).unwrap();
    let dir = fs::read_dir(&c.directory)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(
        &f.job.modern_path,
        b"operator edit must not alter the recovery payload",
    )
    .unwrap();
    assert_eq!(bounded_hash(&dir.join("payload")).unwrap(), sha);
    assert!(c.replay().is_err());
    // Fixture-owned output removal simulates an interrupted/missing final copy.
    fs::remove_file(&f.job.modern_path).unwrap();
    assert_eq!(c.replay().unwrap(), Some(sha.clone()));
    assert_eq!(bounded_hash(&f.job.modern_path).unwrap(), sha);
}

#[test]
fn competing_sealed_payloads_never_select_a_filesystem_order_winner() {
    let f = Fixture::new();
    let c = f.context();
    let stop = |s: &str| {
        if s == "sealed" {
            Err("exit".into())
        } else {
            Ok(())
        }
    };
    assert!(c.publish(&f.made, &stop).is_err());
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&f.made)
        .unwrap();
    let mut zip = zip::ZipWriter::new_append(file).unwrap();
    zip.start_file(
        "fixture-addition.xml",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(b"a distinct structurally valid output")
        .unwrap();
    zip.finish().unwrap().sync_all().unwrap();
    assert!(c.publish(&f.made, &stop).is_err());
    assert!(c.replay().unwrap_err().contains("Competing"));
    assert!(!f.job.modern_path.exists());
    assert_eq!(fs::read_dir(&c.directory).unwrap().count(), 2);
}

#[test]
fn cancellation_after_sealing_keeps_an_offline_replayable_output() {
    let f = Fixture::new();
    let c = f.context();
    let token = crate::cancellation::Token::default();
    let scope = crate::cancellation::enter(token.clone());
    let error = c
        .publish(&f.made, &|s| {
            if s == "sealed" {
                token.request();
            }
            Ok(())
        })
        .unwrap_err();
    assert!(crate::cancellation::stopped(&error));
    assert!(!f.job.modern_path.exists());
    drop(scope);
    assert_eq!(c.replay().unwrap(), Some(bounded_hash(&f.made).unwrap()));
}

#[test]
fn source_edit_after_sealing_refuses_before_target_promotion() {
    let f = Fixture::new();
    let c = f.context();
    let error = c
        .publish(&f.made, &|s| {
            if s == "sealed" {
                fs::write(&f.job.source_path, b"changed after seal").unwrap();
            }
            Ok(())
        })
        .unwrap_err();
    assert!(error.contains("source changed"));
    assert!(!f.job.modern_path.exists());
    assert!(c.replay().is_err());
    assert_eq!(fs::read(&f.job.source_path).unwrap(), b"changed after seal");
}

#[test]
fn foreign_and_oversized_receipts_refuse_without_publication() {
    for oversized in [false, true] {
        let f = Fixture::new();
        let c = f.context();
        assert!(
            c.publish(&f.made, &|s| if s == "sealed" {
                Err("exit".into())
            } else {
                Ok(())
            })
            .is_err()
        );
        let path = fs::read_dir(&c.directory)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path()
            .join("receipt.json");
        if oversized {
            fs::write(path, vec![b' '; 65537]).unwrap();
        } else {
            let mut r: Receipt = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            r.binding.project = f.root.join("other-project");
            fs::write(path, serde_json::to_vec(&r).unwrap()).unwrap();
        }
        assert!(c.replay().is_err());
        assert!(!f.job.modern_path.exists());
    }
}
