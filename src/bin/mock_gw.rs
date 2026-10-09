use std::{env, fs, path::PathBuf, thread, time::Duration};

// Match real gw.exe: ordinary messages and sector grids use stderr.
macro_rules! println {
    ($($arg:tt)*) => { std::eprintln!($($arg)*); };
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "mock-worker") {
        let root = PathBuf::from(env::var_os("MOCK_GW_TREE_ROOT").unwrap());
        fs::write(root.join("worker.pid"), std::process::id().to_string()).unwrap();
        for beat in 0..6000 {
            fs::write(root.join("worker.beat"), beat.to_string()).unwrap();
            thread::sleep(Duration::from_millis(20));
        }
        return;
    }

    if args.first().is_some_and(|arg| arg == "--version") {
        println!("mock_gw 1.23");
        return;
    }
    // Saved-file finalization fixture: satisfy the 7-Zip health probe, then
    // deliberately leave extraction to FluxVault's native FAT12 fallback.
    if args.as_slice() == ["i"] && env::var_os("MOCK_GW_SEVENZIP_PROBE").is_some() {
        println!("7-Zip 24.09 mock saved-file fixture");
        return;
    }
    // Disposable Office-runner lifecycle fixture; never opens hardware.
    if args.first().is_some_and(|arg| arg == "--headless") {
        use std::io::Write;
        let out = PathBuf::from(&args[args.iter().position(|s| s == "--outdir").unwrap() + 1]);
        let format = &args[args.iter().position(|s| s == "--convert-to").unwrap() + 1];
        let extension = format.split(':').next().unwrap();
        let source = std::path::Path::new(args.last().unwrap());
        let target = out.join(format!(
            "{}.{}",
            source.file_stem().unwrap().to_string_lossy(),
            extension
        ));
        if env::var_os("MOCK_GW_OFFICE_TREE").is_some() {
            mock_tree(Some(&target));
        }
        if extension == "pdf" {
            fs::write(
                target,
                b"%PDF-1.7\nmock fixture, not a real conversion\n%%EOF\n",
            )
            .unwrap();
        } else {
            let mut archive = zip::ZipWriter::new(fs::File::create(target).unwrap());
            for (name, bytes) in [
                ("[Content_Types].xml", b"<Types/>".as_slice()),
                (
                    "word/document.xml",
                    b"<document>mock conversion fixture</document>".as_slice(),
                ),
            ] {
                archive
                    .start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                archive.write_all(bytes).unwrap();
            }
            archive.finish().unwrap();
        }
        return;
    }

    if args.is_empty() || args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("mock_gw: Mock Greaseweazle host tool for FluxVault testing");
        println!("Supported commands: info, read, convert");
        return;
    }

    // Hang simulation for timeouts
    if env::var("MOCK_GW_HANG").is_ok() || args.iter().any(|arg| arg == "--mock-hang") {
        thread::sleep(Duration::from_secs(60));
        return;
    }

    // Simulated failure
    if env::var("MOCK_GW_FAIL").is_ok() || args.iter().any(|arg| arg == "--mock-fail") {
        eprintln!("** ERROR: mock_gw simulated failure");
        std::process::exit(1);
    }

    match args[0].as_str() {
        "info" => {
            if env::var_os("MOCK_GW_INFO_TREE").is_some() {
                mock_tree(None);
            }
            if env::var("MOCK_GW_DEVICE_NOT_FOUND").is_ok()
                || args.iter().any(|arg| arg == "--mock-not-found")
            {
                println!("Host Tools: 1.23");
                println!("Device:");
                println!("  Not found");
                return;
            }

            if env::var("MOCK_GW_BOOTLOADER").is_ok()
                || args.iter().any(|arg| arg == "--mock-bootloader")
            {
                println!("Host Tools: 1.23");
                println!("Device:");
                println!("  Port: COM3");
                println!("  Model: Greaseweazle V4");
                println!("  Firmware: 1.23 (Bootloader)");
                return;
            }

            println!("Host Tools: 1.23");
            println!("Device:");
            println!("  Port: COM3");
            println!("  Model: Greaseweazle V4");
            println!("  Firmware: 1.23");
        }
        "read" => {
            let no_index_attempt =
                env::var("MOCK_GW_NO_INDEX_ATTEMPTS")
                    .ok()
                    .is_some_and(|targets| {
                        let name = args
                            .last()
                            .and_then(|a| std::path::Path::new(a).file_name())
                            .unwrap_or_default()
                            .to_string_lossy();
                        targets
                            .split(',')
                            .any(|stem| !stem.is_empty() && name == format!("{stem}.partial.scp"))
                    });
            if env::var("MOCK_GW_NO_INDEX").is_ok() || no_index_attempt {
                if env::var("MOCK_GW_NO_INDEX_PARTIAL").is_ok() {
                    fs::write(args.last().unwrap(), b"synthetic interrupted raw flux").unwrap();
                }
                println!("Command Failed: GetFluxStatus: No Index");
                return; // Reproduce the real host's misleading exit-zero behavior.
            }
            let output_path = match args.last() {
                Some(path) => PathBuf::from(path),
                None => {
                    eprintln!("** ERROR: No output path specified for read");
                    std::process::exit(1);
                }
            };
            if env::var_os("MOCK_GW_READ_TREE").is_some() {
                mock_tree(Some(&output_path));
            }

            let revolutions = args
                .iter()
                .find_map(|arg| arg.strip_prefix("--revs="))
                .and_then(|r| r.parse::<u32>().ok())
                .unwrap_or(3);

            println!("Reading c=0-79:h=0-1 revs={revolutions}");
            for cylinder in 0..2 {
                for head in 0..2 {
                    println!("T{cylinder}.{head}: Raw Flux (28123 flux in 200.1ms)");
                }
            }

            let cylinders = args
                .iter()
                .find_map(|a| a.strip_prefix("--tracks=c="))
                .map(|value| {
                    value
                        .split(':')
                        .next()
                        .unwrap()
                        .split(',')
                        .map(|c| c.parse::<usize>().unwrap())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| (0..80).collect());
            let bad = if env::var("MOCK_GW_RECOVER_AFTER_FAST").is_ok() && revolutions == 2 {
                vec![24]
            } else {
                env::var("MOCK_GW_BAD_LBAS")
                    .unwrap_or_default()
                    .split(',')
                    .filter_map(|v| v.parse::<usize>().ok())
                    .collect::<Vec<_>>()
            };
            let conflict = if revolutions > 2 {
                env::var("MOCK_GW_CONFLICT_LBA")
                    .ok()
                    .and_then(|v| v.parse::<usize>().ok())
            } else {
                None
            };
            let media_format = env::var("MOCK_GW_MEDIA_FORMAT").unwrap_or_else(|_| {
                args.iter()
                    .find_map(|a| a.strip_prefix("--format="))
                    .unwrap_or("ibm.1440")
                    .to_owned()
            });
            // Optional disposable filesystem bytes travel inside the synthetic
            // capture, so later/offline converts do not depend on a source path.
            let image = env::var_os("MOCK_GW_IMAGE").map(|path| fs::read(path).unwrap());
            let fixture = serde_json::json!({"cylinders":cylinders,"bad":bad,"conflict":conflict,"media_format":media_format,"image":image});
            // Synthetic capture carries scenario/coverage into a later CLI process.
            if let Err(e) = fs::write(&output_path, serde_json::to_vec(&fixture).unwrap()) {
                eprintln!("** ERROR: Failed to write {}: {e}", output_path.display());
                std::process::exit(1);
            }
        }
        "convert" => {
            if env::var_os("MOCK_GW_CONVERT_TREE").is_some() {
                mock_tree(Some(std::path::Path::new(args.last().unwrap())));
            }
            if env::var("MOCK_GW_FAIL_CONVERT").is_ok() {
                eprintln!("** ERROR: simulated interrupted decode");
                std::process::exit(1);
            }
            let output_path = match args.last() {
                Some(path) => PathBuf::from(path),
                None => {
                    eprintln!("** ERROR: No output path specified for convert");
                    std::process::exit(1);
                }
            };

            let is_720 = args.iter().any(|arg| arg == "--format=ibm.720");
            let sectors_per_track = if is_720 { 9 } else { 18 };
            let total_sectors = 80 * 2 * sectors_per_track;
            let total_bytes = total_sectors * 512;
            let fixture: serde_json::Value =
                serde_json::from_slice(&fs::read(&args[args.len() - 2]).unwrap()).unwrap();
            let cylinders = fixture["cylinders"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as usize)
                .collect::<Vec<_>>();
            let mut bad = fixture["bad"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as usize)
                .collect::<Vec<_>>();
            if fixture["media_format"]
                .as_str()
                .is_some_and(|p| p != if is_720 { "ibm.720" } else { "ibm.1440" })
            {
                bad = (0..total_sectors).collect();
            }

            println!("Converting input -> {}", output_path.display());
            println!("Reading c=0-79:h=0-1");
            println!("T0.0: IBM-MFM ({sectors_per_track}/{sectors_per_track} sectors)");
            println!("T0.1: IBM-MFM ({sectors_per_track}/{sectors_per_track} sectors)");

            // Print full 80-cylinder grid
            let tens: String = (0..80)
                .map(|c| {
                    if c % 10 == 0 {
                        char::from_digit((c / 10) as u32, 10).unwrap()
                    } else {
                        ' '
                    }
                })
                .collect();
            let units: String = (0..80)
                .map(|c| char::from_digit((c % 10) as u32, 10).unwrap())
                .collect();
            println!("Cyl-> {tens}");
            println!("H. S: {units}");
            for head in 0..2 {
                for sector in 0..sectors_per_track {
                    let dots: String = (0..80)
                        .map(|c| {
                            let lba = (c * 2 + head) * sectors_per_track + sector;
                            if !cylinders.contains(&c) {
                                ' '
                            } else if bad.contains(&lba) {
                                'X'
                            } else {
                                '.'
                            }
                        })
                        .collect();
                    println!("{head}.{sector:>2}: {dots}");
                }
            }
            let covered = cylinders.len() * 2 * sectors_per_track;
            let missing = bad
                .iter()
                .filter(|lba| cylinders.contains(&(**lba / (sectors_per_track * 2))))
                .count();
            println!("Found {} sectors of {covered}", covered - missing);

            // Write dummy disk image
            let provided: Option<Vec<u8>> =
                serde_json::from_value(fixture["image"].clone()).unwrap();
            let has_fixture = provided.is_some();
            let mut image = provided.unwrap_or_else(|| vec![0; total_bytes]);
            assert_eq!(
                image.len(),
                total_bytes,
                "mock filesystem geometry mismatch"
            );
            for lba in 0..total_sectors {
                if !cylinders.contains(&(lba / (sectors_per_track * 2))) || bad.contains(&lba) {
                    image[lba * 512..(lba + 1) * 512].fill(0);
                } else if !has_fixture {
                    image[lba * 512..(lba + 1) * 512].fill(0xE5);
                }
            }
            if let Some(lba) = fixture["conflict"].as_u64() {
                image[lba as usize * 512] = 0x99;
            }
            if let Err(e) = fs::write(&output_path, image) {
                eprintln!("** ERROR: Failed to write {}: {e}", output_path.display());
                std::process::exit(1);
            }
        }
        other => {
            eprintln!("** ERROR: Command not supported by mock_gw: {other}");
            std::process::exit(1);
        }
    }
}

// Lifecycle fixture only: no device calls. Descendant deliberately inherits
// output pipes, exercising cleanup when the leader exits before its child.
fn mock_tree(partial: Option<&std::path::Path>) {
    let root = PathBuf::from(env::var_os("MOCK_GW_TREE_ROOT").unwrap());
    fs::create_dir_all(&root).unwrap();
    if let Some(path) = partial {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .unwrap();
        file.write_all(b"interrupted mock capture evidence")
            .unwrap();
        file.sync_all().unwrap();
    }
    let child = std::process::Command::new(env::current_exe().unwrap())
        .arg("mock-worker")
        .spawn()
        .unwrap();
    let begun = std::time::Instant::now();
    while !root.join("worker.pid").exists() {
        assert!(begun.elapsed() < Duration::from_secs(10));
        thread::sleep(Duration::from_millis(10));
    }
    fs::write(
        root.join("tree.json"),
        serde_json::json!({"leader":std::process::id(),"worker":child.id()}).to_string(),
    )
    .unwrap();
    if partial.is_some() {
        println!("T0.0: mock capture active, waiting for controller interruption");
        thread::sleep(Duration::from_secs(120));
        std::process::exit(1);
    }
}
