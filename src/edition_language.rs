//! Translate only generated presentation. Source strings occupy opaque slots
//! until the final one-pass expansion; inserted data is never translated again.
use crate::final_report::Language;

#[derive(Default)]
pub(crate) struct Slots(Vec<String>);
impl Slots {
    pub(crate) fn protect(&mut self, escaped: String) -> String {
        let token = format!("__FV_LOCALE_SLOT_{}__", self.0.len());
        self.0.push(escaped);
        token
    }
    pub(crate) fn expand(&self, template: &str) -> Result<String, String> {
        let mut out = String::new();
        let mut rest = template;
        const PREFIX: &str = "__FV_LOCALE_SLOT_";
        while let Some(start) = rest.find(PREFIX) {
            out.push_str(&rest[..start]);
            let token = &rest[start + PREFIX.len()..];
            let end = token
                .find("__")
                .ok_or("Invalid edition localization slot")?;
            let index = token[..end]
                .parse::<usize>()
                .map_err(|_| "Invalid edition localization slot")?;
            out.push_str(
                self.0
                    .get(index)
                    .ok_or("Unknown edition localization slot")?,
            );
            if out.len() > crate::fat12::MAX_IMAGE_BYTES {
                return Err("Localized edition size ceiling; text retained".into());
            }
            rest = &token[end + 2..];
        }
        out.push_str(rest);
        if out.len() > crate::fat12::MAX_IMAGE_BYTES {
            return Err("Localized edition size ceiling; text retained".into());
        }
        Ok(out)
    }
}

pub(crate) fn hungarian_markup(mut template: String) -> String {
    // The caller MUST first mask all source/dynamic strings with Slots. These
    // replacements act on renderer-owned markup/prose, never recovered content.
    for (en, hu) in [
        ("<html lang=\"en\">", "<html lang=\"hu\">"),
        (
            "FluxVault — recovered text edition",
            "FluxVault — kimentett szövegkiadás",
        ),
        (
            "FluxVault — recovered RTF prefix",
            "FluxVault — kimentett RTF-előtag",
        ),
        (
            "FLUXVAULT / FORENSIC RECOVERY EDITION",
            "FLUXVAULT / BIZONYÍTÉKALAPÚ SZÖVEGKIMENTÉS",
        ),
        (
            "FLUXVAULT / FORENSIC RTF PREFIX EDITION",
            "FLUXVAULT / BIZONYÍTÉKALAPÚ RTF-ELŐTAG",
        ),
        (
            "Readable main-text segments from an incomplete Word document.",
            "Hiányos Word-dokumentum olvasható főszövegrészei.",
        ),
        ("Source image:", "Forráslemez-kép:"),
        ("Source:", "Forrás:"),
        (
            "Declared main-text positions:",
            "Rögzített főszövegpozíciók:",
        ),
        ("Readable:", "Olvasható:"),
        ("Missing/invalid:", "Hiányzó/érvénytelen:"),
        (
            "Generated edition — not the original document",
            "Előállított kiadás — nem az eredeti dokumentum",
        ),
        (
            "Only source-mapped readable text is shown. Missing positions are marked, never guessed. Segment boundaries remain visible. Formatting, tables, pictures, fields, revisions and original meaning are not reconstructed or certified. This HTML is a new presentation of forensic text, not a repaired DOC.",
            "Csak a forráshoz visszaköthető, olvasható szöveg jelenik meg. A hiányzó pozíciók jelölve vannak; nem találgatjuk a tartalmukat. A szövegrészek határai láthatók. A formázás, táblázatok, képek, mezők, módosítások és az eredeti jelentés nem kerültek helyreállításra vagy hitelesítésre. Ez a HTML a kimentett szöveg új megjelenítése, nem javított DOC.",
        ),
        (
            "CP ranges are zero-based Word character positions (UTF-16 code units). Non-whitespace control characters are displayed as literal Unicode escape labels. Consult word-text.json and the separate text files for original decoded characters and exact source extents.",
            "A CP-tartományok nullától számozott Word-karakterpozíciók (UTF-16 kódegységek). A nem térköz jellegű vezérlőkarakterek Unicode escape-jelölésként láthatók. Az eredeti dekódolt karakterek és pontos forrástartományok a külön szövegfájlokban és a word-text.json jelentésben találhatók.",
        ),
        (
            "Content-based format detection",
            "Tartalom alapú formátumfelismerés",
        ),
        (
            "Partial compound-file metadata",
            "Hiányos összetettfájl-metaadatok",
        ),
        (
            " unavailable directory entries; ",
            " nem olvasható könyvtárbejegyzés; ",
        ),
        (
            " unrelated allocation exceptions. Entire original container remains unverified.",
            " független kiosztási kivétel. Az eredeti tároló teljes épsége nem igazolt.",
        ),
        (
            "Unused storage fields retained",
            "Nem használt tárolómezők megőrizve",
        ),
        (
            " storage entries contain nonzero start/size fields. These fields do not describe streams for storage objects; their original bytes are recorded in word-text.json, not followed as allocation or rewritten. Required stream, root mini-stream and sibling-tree checks remain enforced.",
            " tárolóbejegyzésben nem nulla kezdőszektor/méret szerepel. Ezek a mezők tárolóobjektumoknál nem adatfolyam-kiosztást jelentenek; eredeti bájtjaikat a word-text.json megőrzi, nem követjük vagy írjuk át őket. A szükséges adatfolyam-, gyökér-mini-adatfolyam- és testvérfa-ellenőrzések továbbra is kötelezők.",
        ),
        (
            "Incomplete Word header — text-only candidate",
            "Hiányos Word-fejléc — csak szöveges jelölt",
        ),
        (
            "Unavailable extension count at Word stream byte ",
            "Nem olvasható bővítményszám a Word-adatfolyam következő bájtjánál: ",
        ),
        (
            ". Supported-header exclusion ends at byte ",
            ". A támogatott fejlécek kizárási határának bájtpozíciója: ",
        ),
        (
            "; this is a conservative search bound, not a reconstructed header.",
            "; ez óvatos keresési korlát, nem helyreállított fejléc.",
        ),
        ("READABLE SEGMENT ", "OLVASHATÓ SZÖVEGRÉSZ "),
        (" (end exclusive)", " (a végpont nem része a tartománynak)"),
        (
            "MISSING / INVALID TEXT: CP ",
            "HIÁNYZÓ / ÉRVÉNYTELEN SZÖVEG: CP ",
        ),
        (" positions", " pozíció"),
        (
            "No replacement text has been invented.",
            "Nem készült kitalált helyettesítő szöveg.",
        ),
        (
            "Generated by FluxVault. No active scripts, macros, remote assets or original-document execution. Text fragments/editions do not count as recovered whole files or certified customer documents.",
            "A FluxVault által előállított kiadás. Nem futtat scripteket, makrókat vagy eredeti dokumentumkódot, és nem tölt be távoli tartalmat. A szövegtöredékek/kiadások nem számítanak teljes visszanyert fájlnak vagy hitelesített ügyféldokumentumnak.",
        ),
        (
            "Generated partial text — not a repaired RTF",
            "Előállított részleges szöveg — nem javított RTF",
        ),
        ("Readable prefix text", "Olvasható előtagszöveg"),
        (
            "STOPPED / REMAINDER NOT INTERPRETED",
            "MEGÁLLT / A FENNMARADÓ RÉSZ NEM ÉRTELMEZETT",
        ),
        (
            " readable UTF-16 pozíció · ",
            " olvasható UTF-16 pozíció · ",
        ),
        (
            " input prefix bytes · decoded through file byte ",
            " bemeneti előtagbájt · értelmezési végpont (fájlbájt): ",
        ),
        (
            " file bytes remain uninterpreted. Their text length is unknown; no replacement words or formatting state have been guessed.",
            " fájlbájt maradt értelmezetlen. Szöveghosszuk ismeretlen; nem találgatjuk a hiányzó szavakat vagy formázási állapotot.",
        ),
        (
            "UNINTERPRETED SYMBOL-FONT GLYPH",
            "NEM ÉRTELMEZETT SZIMBÓLUMKARAKTER",
        ),
        ("Readable file bytes ", "Olvasható fájlbájtok: "),
        (", font ", ", betűtípus-azonosító: "),
        (", encoded byte ", ", kódolt bájt: "),
        (
            ". Glyph meaning unknown; no replacement character or word has been invented.",
            ". A karakter jelentése ismeretlen; nem készült kitalált helyettesítő karakter vagy szó.",
        ),
        (
            "Next independently retained text segment",
            "Következő, külön megőrzött szövegrész",
        ),
    ] {
        template = template.replace(en, hu);
    }
    template
}

pub(crate) fn source_language(source: &std::path::Path) -> Result<Language, String> {
    let root = source
        .parent()
        .and_then(std::path::Path::parent)
        .ok_or("Recovery source has no project root")?;
    if !root
        .join("project.json")
        .try_exists()
        .map_err(|e| e.to_string())?
    {
        return Ok(Default::default());
    }
    crate::project_settings::language_at(root)
}

pub(crate) fn stop_hu(reason: &str) -> String {
    match reason {
        "RTF control delimiter unavailable" => "Az RTF vezérlőelem lezárása nem olvasható; a hiányzó bájt nem lett kitalálva.".into(),
        "First acquisition-unreadable sector; no parser resynchronization" => "Az első nem olvasható beolvasási szektornál megállt az értelmezés; a hiányon át nem találgatjuk az állapotot.".into(),
        "First allocation-unmapped byte; no parser resynchronization" => "Az első nem leképezett fájlbájtnál megállt az értelmezés; a hiányon át nem találgatjuk az állapotot.".into(),
        "Readable root group ended; original completeness remains unverified" => "Az olvasható gyökércsoport véget ért; az eredeti dokumentum teljessége továbbra sem igazolt.".into(),
        "Readable-prefix input ceiling; search not exhaustive" => "Elértük az olvasható előtag feldolgozási korlátját; a vizsgálat nem teljes körű.".into(),
        _ => "A további szöveg vagy értelmezési állapot nem igazolható, ezért a kimentés megállt. A pontos műszaki okot az rtf-text.json tartalmazza; nem készült kitalált tartalom.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_slots_are_opaque_even_when_text_looks_like_markup_or_another_slot() {
        let mut slots = Slots::default();
        let source = "READABLE SEGMENT __FV_LOCALE_SLOT_1__ &lt;script&gt;";
        let token = slots.protect(source.into());
        slots.protect("must not replace source's literal marker".into());
        let template = hungarian_markup(format!("<html lang=\"en\">READABLE SEGMENT {token}"));
        assert_eq!(
            slots.expand(&template).unwrap(),
            format!("<html lang=\"hu\">OLVASHATÓ SZÖVEGRÉSZ {source}")
        );
    }

    #[test]
    fn expansion_checks_slots_and_actual_expanded_size() {
        let mut slots = Slots::default();
        assert!(slots.expand("__FV_LOCALE_SLOT_0__").is_err());
        assert!(slots.expand("__FV_LOCALE_SLOT_bad__").is_err());
        assert!(slots.expand("__FV_LOCALE_SLOT_0").is_err());
        let token = slots.protect("x".repeat(crate::fat12::MAX_IMAGE_BYTES));
        assert!(slots.expand(&format!("{token}x")).is_err());
        assert!(
            slots
                .expand(&"x".repeat(crate::fat12::MAX_IMAGE_BYTES + 1))
                .is_err()
        );
    }
}
