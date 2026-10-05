use std::{env, fs, path::PathBuf, thread, time::Duration};

// Match real gw.exe: ordinary messages and sector grids use stderr.
macro_rules! println {
    ($($arg:tt)*) => { std::eprintln!($($arg)*); };
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

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
            let output_path = match args.last() {
                Some(path) => PathBuf::from(path),
                None => {
                    eprintln!("** ERROR: No output path specified for read");
                    std::process::exit(1);
                }
            };

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
            let fixture = serde_json::json!({"cylinders":cylinders,"bad":bad,"conflict":conflict});
            // Synthetic capture carries scenario/coverage into a later CLI process.
            if let Err(e) = fs::write(&output_path, serde_json::to_vec(&fixture).unwrap()) {
                eprintln!("** ERROR: Failed to write {}: {e}", output_path.display());
                std::process::exit(1);
            }
        }
        "convert" => {
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
            let bad = fixture["bad"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as usize)
                .collect::<Vec<_>>();

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
            let mut image = vec![0; total_bytes];
            for lba in 0..total_sectors {
                if cylinders.contains(&(lba / (sectors_per_track * 2))) && !bad.contains(&lba) {
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
