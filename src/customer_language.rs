//! Presentation-only language. Original names, stable codes, JSON/CSV evidence,
//! customer bytes and raw tool diagnostics are never rewritten by this layer.
use crate::final_report::Language;

pub(crate) fn warning_summary(language: Language, raw: &str) -> String {
    if language == Language::English || raw.is_empty() || raw == "[]" {
        return raw.to_owned();
    }
    let lower = raw.to_ascii_lowercase();
    if lower.contains("unreadable")
        || lower.contains("missing sector")
        || lower.contains("disputed")
        || lower.contains("bad sector")
    {
        "Olvasási hiány vagy ellentmondás miatt egyes adatok nem igazoltak. A pontos tartományok és részletek a műszaki JSON/CSV-mellékletben találhatók.".into()
    } else if lower.contains("conversion")
        || lower.contains("libreoffice")
        || lower.contains("pdf")
        || lower.contains("ooxml")
    {
        "A dokumentumkonverzió vagy a létrehozott másolat ellenőrzése figyelmet igényel. A konverzió nem igazol teljes helyreállítást; a pontos ok a műszaki JSON/CSV-mellékletben található.".into()
    } else if lower.contains("filesystem")
        || lower.contains("fat12")
        || lower.contains("bpb")
        || lower.contains("unsupported format")
    {
        "A fájlrendszer vagy lemezformátum vizsgálata korlátozott eredményt adott. Ez nem bizonyít üres vagy teljesen visszanyert lemezt; a részletek a műszaki mellékletben találhatók.".into()
    } else if lower.contains("untracked")
        || lower.contains("manifest")
        || lower.contains("inventory")
        || lower.contains("unverified")
        || lower.contains("binding")
    {
        "A fájljegyzék vagy a forráshoz kötött bizonyíték ellenőrzése figyelmet igényel. Az érintett fájlok és kötések a műszaki JSON/CSV-mellékletben találhatók.".into()
    } else {
        "Az integritásvizsgálat figyelmeztetést jelzett. A pontos eredeti üzenet és az érintett fájlok a műszaki JSON/CSV-mellékletben találhatók; teljes helyreállítás nem igazolt.".into()
    }
}
pub(crate) fn cell(language: Language, column: &str, raw: &str) -> String {
    if language == Language::English || raw.is_empty() {
        return raw.into();
    }
    match column {
        "NativeFilesystemStatus" => match raw {
            "NO REACHABLE LIVE FAT FILES; DELETED ENTRIES PRESENT" => "Nincs elérhető élő FAT-fájl; törölt bejegyzések jelen vannak, visszanyerhetőségük nem igazolt",
            "NO REACHABLE LIVE FAT FILES; CONTENT COMPLETENESS UNPROVEN" => "Nincs elérhető élő FAT-fájl; a tartalom teljessége nem igazolt",
            "FAT FILE/DIRECTORY GAPS OR AMBIGUITY" => "Hiányos vagy többértelmű FAT-fájl- vagy könyvtárleképezés",
            "SUPPORTED FAT ANALYSIS; HISTORICAL COMPLETENESS UNPROVEN" => "Támogatott FAT-vizsgálat; a történeti teljesség nem igazolt",
            "UNSUPPORTED/UNKNOWN FILESYSTEM; CARVING DOES NOT ESTABLISH LIVE OWNERSHIP" => "Nem támogatott vagy ismeretlen fájlrendszer; a szignatúrás kimentés nem igazolja az eredeti élő fájltulajdont",
            _ => "A fájlrendszer állapota nem igazolt; pontos műszaki leírás a JSON/CSV-mellékletben",
        }.into(),
        "Issues"|"Detail"|"Details"=>warning_summary(language,raw),
        "RecoveryMethod"=>match raw {
            "Filesystem recovery"=>"Fájlrendszer alapján visszanyert",
            "Native FAT12 readable-chain recovery; filesystem completeness unverified"=>"Olvasható FAT12-lánc alapján visszanyert; a fájlrendszer teljessége nem igazolt",
            "DMDE filesystem recovery; artifact folders removed"=>"Korábbi DMDE fájlrendszer-alapú kimentés; segédmappanevek eltávolítva",
            "Signature recovered; original filename unavailable"=>"Fájlszignatúra alapján visszanyert; az eredeti fájlnév ismeretlen",
            "Reconstructed directory; original parent/name and live/deleted ownership unknown"=>"Rekonstruált könyvtár; az eredeti szülő, név és élő/törölt állapot ismeretlen",
            "Fragment-chain hypothesis; structurally validated, original missing FAT link unproven"=>"Töredéklánc-feltételezés; szerkezetileg ellenőrzött, az eredeti hiányzó FAT-kapcsolat nem igazolt",
            _=>return "Visszanyerési módszer műszaki bizonyítékkal rögzítve; pontos leírás a JSON/CSV-mellékletben".into(),
        }.into(),
        _=>raw.into(),
    }
}

/// Descriptions supplement, never replace stable technical status identifiers.
pub(crate) fn status_description(language: Language, code: &str) -> &'static str {
    let (hu, en) = if code == "OK" {
        (
            "A rögzített eredmények integritásellenőrzése sikeres; az eredeti tartalom teljessége nem igazolt",
            "Recorded results passed integrity checks; original completeness is not certified",
        )
    } else if code.contains("CONVERSION") {
        (
            "A konverzió figyelmet igényel; nem igazolt teljes dokumentumhelyreállítás",
            "Conversion needs attention; complete document recovery is not established",
        )
    } else if code.contains("NO REACHABLE") || code.contains("NO RECOVERED") {
        (
            "Nincs igazoltan elérhető teljes forrásfájl; ez nem bizonyít üres lemezt",
            "No verified reachable whole source files; this does not establish an empty disk",
        )
    } else if code.contains("UNSUPPORTED") {
        (
            "Nem támogatott vagy ismeretlen fájlrendszer; a megőrzött bizonyíték külön vizsgálható",
            "Unsupported or unknown filesystem; preserved evidence remains available for investigation",
        )
    } else if code.contains("IMAGE READ") {
        (
            "A lemezkép olvasási hiányokat tartalmaz; a függetlenül ép fájlok ettől még visszanyerhetők",
            "The image has read gaps; independently intact files may still be recovered",
        )
    } else if code.contains("DERIVED") {
        (
            "Származtatott helyreállítási eredmény; nem változatlan eredeti és nem teljességi tanúsítvány",
            "Derived recovery evidence; not an unchanged original or a completeness certificate",
        )
    } else {
        (
            "Az ellenőrzés figyelmet igényel; az eredeti műszaki bizonyíték a JSON/CSV-mellékletben található",
            "Checks need attention; original technical evidence is in the JSON/CSV annex",
        )
    };
    language.text(hu, en)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn human_warnings_are_hungarian_but_english_and_raw_paths_are_exact() {
        for raw in [
            "Unreadable input sector 22",
            "LibreOffice failed on test.doc",
            "FAT12 filesystem unavailable",
            "Untracked output",
            "Arbitrary raw tool warning <script>",
        ] {
            let hu = warning_summary(Language::Hungarian, raw);
            assert!(hu.contains("melléklet"));
            assert!(!hu.contains("<script>"));
            assert_eq!(warning_summary(Language::English, raw), raw);
        }
        for column in [
            "OriginalForensicPath",
            "ModernPath",
            "SourceSHA256",
            "Status",
            "Image",
        ] {
            assert_eq!(
                cell(Language::Hungarian, column, "Ügyfél/original.doc"),
                "Ügyfél/original.doc"
            );
        }
        assert_eq!(cell(Language::Hungarian, "Issues", "[]"), "[]");
    }

    #[test]
    fn native_descriptions_are_localized_but_codes_and_unknown_source_names_stay_literal() {
        for raw in [
            "NO REACHABLE LIVE FAT FILES; DELETED ENTRIES PRESENT",
            "NO REACHABLE LIVE FAT FILES; CONTENT COMPLETENESS UNPROVEN",
            "FAT FILE/DIRECTORY GAPS OR AMBIGUITY",
            "SUPPORTED FAT ANALYSIS; HISTORICAL COMPLETENESS UNPROVEN",
            "UNSUPPORTED/UNKNOWN FILESYSTEM; CARVING DOES NOT ESTABLISH LIVE OWNERSHIP",
            "future unsupported description",
        ] {
            assert_ne!(
                cell(Language::Hungarian, "NativeFilesystemStatus", raw),
                raw
            );
            assert_eq!(cell(Language::English, "NativeFilesystemStatus", raw), raw);
        }
        for code in [
            "OK",
            "PARTIAL: IMAGE READ",
            "CHECK: CONVERSION FAILED",
            "CHECK: NO REACHABLE LIVE FILES",
            "CHECK: DERIVED RECOVERY",
            "unknown",
        ] {
            assert!(!status_description(Language::Hungarian, code).is_empty());
            assert_ne!(
                status_description(Language::Hungarian, code),
                status_description(Language::English, code)
            );
            assert_eq!(cell(Language::Hungarian, "AuditStatus", code), code);
        }
        assert_eq!(cell(Language::Hungarian, "NativeFilesystemStatus", ""), "");
        assert_eq!(
            cell(
                Language::Hungarian,
                "OriginalForensicPath",
                "NO REACHABLE LIVE FAT FILES; DELETED ENTRIES PRESENT"
            ),
            "NO REACHABLE LIVE FAT FILES; DELETED ENTRIES PRESENT"
        );
    }
}
