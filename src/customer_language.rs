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
    if language == Language::English {
        return raw.into();
    }
    match column {
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
}
