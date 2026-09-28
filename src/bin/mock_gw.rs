use std::{env, fs, path::PathBuf, thread, time::Duration};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() || args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("mock_gw: Mock Greaseweazle host tool for FluxVault testing");
        println!("Supported commands: --version, info, read, convert");
        return;
    }

    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("Greaseweazle Tools v1.23");
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

            // Write mock SCP file
            if let Err(e) = fs::write(
                &output_path,
                b"SCP synthetic raw flux capture for mock testing",
            ) {
                eprintln!("** ERROR: Failed to write {}: {e}", output_path.display());
                std::process::exit(1);
            }
        }
        "convert" => {
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
                    let dots = ".".repeat(80);
                    println!("{head}.{sector:>2}: {dots}");
                }
            }
            println!("Found {total_sectors} sectors of {total_sectors} (100%)");

            // Write dummy disk image
            if let Err(e) = fs::write(&output_path, vec![0xE5; total_bytes]) {
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
