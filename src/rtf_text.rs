//! Conservative RTF prefix decoding. Never resume state across an unknown byte.
//! MSFT-RTF syntax/group state; this bounded subset is not a rendering engine.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const MAX_TEXT: usize = 200_000;
const MAX_DEPTH: usize = 128;
const MAX_GAPS: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct GlyphGap {
    pub input_start: usize,
    pub input_end: usize,
    pub output_utf8_offset: usize,
    pub font_id: i32,
    pub encoded_byte: u8,
}

pub(super) struct Decoded {
    pub text: String,
    pub through: usize,
    pub stop: String,
    pub gaps: Vec<GlyphGap>,
}
#[derive(Clone)]
struct State {
    skip: bool,
    hidden: bool,
    deleted: bool,
    fonts: bool,
    font: Option<i32>,
    uc: usize,
    start: bool,
}
#[derive(Default)]
struct Font {
    charset: Option<i32>,
    cpg: Option<i32>,
}
enum Token<'a> {
    Byte(u8),
    Symbol(u8),
    Word(&'a str, Option<i32>),
}
fn token<'a>(bytes: &'a [u8], at: &mut usize) -> Result<Token<'a>, String> {
    let next = *bytes.get(*at).ok_or("Incomplete RTF escape")?;
    *at += 1;
    if next == b'\'' {
        let pair = bytes.get(*at..*at + 2).ok_or("Incomplete RTF hex escape")?;
        let hex = std::str::from_utf8(pair).map_err(|_| "Invalid RTF hex escape")?;
        let value = u8::from_str_radix(hex, 16).map_err(|_| "Invalid RTF hex escape")?;
        *at += 2;
        return Ok(Token::Byte(value));
    }
    if !next.is_ascii_alphabetic() {
        return Ok(Token::Symbol(next));
    }
    let start = *at - 1;
    while bytes.get(*at).is_some_and(u8::is_ascii_alphabetic) {
        *at += 1;
    }
    if *at - start > 32 {
        return Err("RTF control-word ceiling".into());
    }
    let word = std::str::from_utf8(&bytes[start..*at]).unwrap();
    let number_start = *at;
    if bytes.get(*at) == Some(&b'-') {
        *at += 1;
    }
    while bytes.get(*at).is_some_and(u8::is_ascii_digit) {
        *at += 1;
    }
    let number = if *at > number_start {
        Some(
            std::str::from_utf8(&bytes[number_start..*at])
                .unwrap()
                .parse::<i32>()
                .map_err(|_| "Invalid/oversized RTF parameter")?,
        )
    } else {
        None
    };
    // A hole/end can hide another letter/digit: do not complete that token.
    let delimiter = *bytes.get(*at).ok_or("RTF control delimiter unavailable")?;
    if delimiter == b' ' {
        *at += 1;
    }
    Ok(Token::Word(word, number))
}
fn destination(word: &str) -> bool {
    matches!(
        word,
        "fonttbl"
            | "colortbl"
            | "stylesheet"
            | "filetbl"
            | "info"
            | "pict"
            | "object"
            | "objdata"
            | "shp"
            | "shppict"
            | "nonshppict"
            | "header"
            | "headerl"
            | "headerr"
            | "headerf"
            | "footer"
            | "footerl"
            | "footerr"
            | "footerf"
            | "footnote"
            | "annotation"
            | "field"
            | "fldinst"
            | "fldrslt"
            | "xe"
            | "tc"
            | "listtable"
            | "listoverridetable"
            | "pn"
            | "pntxta"
            | "pntxtb"
            | "generator"
            | "xmlnstbl"
            | "rsidtbl"
    )
}
// Known nontext formatting only. Unknown visible controls STOP, rather than
// treating an unrecognized destination/encoding switch as body text.
fn formatting(word: &str) -> bool {
    if word == "rsidroot" || word == "brdrtbl" {
        return true;
    } // Revision identity metadata, no text.
    const WORDS: &str = "stshfdbch stshfloch stshfhich stshfbi deflang deflangfe deftab fet aenddoc ftntj ftnbj ftnlytwnine sftnbj sftnlytwnine revprop revbar viewkind viewscale viewzk horzdoc paperh paperw margb margl margr margt gutter headery footery colsx sectd sectdefaultcl sectlinegrid sectrsid linex endnhere pgbrdrfoot pgbrdrhead dghorigin dgvorigin dghspace dgvspace dghshow dgvshow dgmargin hyphcaps hyphhotz grfdocevents formshade noxlattoyen nowidctlpar nospaceforul noultrlspc dntblnsbdb expshrtn wrapdefault lytcalctblwd lyttblrtgr lnbrkrule splytwnine htmautsp useltbaln alntblind jexpand aspalpha aspnum nolnhtadjtbl wgrffmtfilter adjustedright adjustright pard s cs ls outlinelevel ql qr qc qj fi li ri lin rin sb sa sl slmult keep keepn widowctrl widctlpar faauto fbias lang langfe langnp langfenp insrsid charrsid pararsid rsid b i ul ulnone ulw uldb uld ulwave strike striked scaps caps cf cb highlight fs expnd expndtw up dn kerning nosupersub super sub chbrdr brdrnone brdrs brdrw brdrcf brsp tx tqc tqr tql tldot tlhyph tlul intbl itap cellx trowd trgaph trleft trrh trftsWidth trftsWidthA trftsWidthB trpaddfb trpaddfl trpaddfr trpaddft trpaddl trpaddr trpaddt trpaddb trbrdrb trbrdrh trbrdrl trbrdrr trbrdrt trbrdrv trkeep trqc trql trqr trhdr lastrow irow irowband clbrdrb clbrdrl clbrdrr clbrdrt clcbpat clcbpatraw clftsWidth clwWidth clvertalt clvertalc clvertalb cltxlrtb clshdrawnil clmgf clmrg clvmgf clvmrg tblind tblindtype ts tsrowd tsvertalt tscellwidthfts tsbrdrb tsbrdrdgl tsbrdrdgr tsbrdrh tsbrdrl tsbrdrr tsbrdrt tsbrdrv chhres hres cgrid nofchars nofcharsws nofpages nofwords vern version red green blue fprq fnil froman fswiss fmodern fscript fdecor ftech fbidi cchs loch hich dbch ltrch rtlch ltrpar rtlpar ltrdoc rtldoc rtlsect ltrsect rtlrow ltrrow deleted revised revdttm revauth revrsid pndec pnhang pnindent pnlcltr pnlcrm pnseclvl pnstart pnucltr pnucrm";
    WORDS.split_ascii_whitespace().any(|w| w == word)
}
fn byte_character(byte: u8, page: Option<i32>) -> Result<char, String> {
    let page = page
        .filter(|p| matches!(p, 1250 | 1252))
        .ok_or("Unsupported/unknown selected font code page")?;
    if byte < 128 {
        if byte < 32 && byte != b'\t' {
            return Err("Unexpected RTF text control byte".into());
        }
        return Ok(byte as char);
    }
    const CP1250: [u16; 128] = [
        0x20ac, 0, 0x201a, 0, 0x201e, 0x2026, 0x2020, 0x2021, 0, 0x2030, 0x0160, 0x2039, 0x015a,
        0x0164, 0x017d, 0x0179, 0, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022, 0x2013, 0x2014, 0,
        0x2122, 0x0161, 0x203a, 0x015b, 0x0165, 0x017e, 0x017a, 0x00a0, 0x02c7, 0x02d8, 0x0141,
        0x00a4, 0x0104, 0x00a6, 0x00a7, 0x00a8, 0x00a9, 0x015e, 0x00ab, 0x00ac, 0x00ad, 0x00ae,
        0x017b, 0x00b0, 0x00b1, 0x02db, 0x0142, 0x00b4, 0x00b5, 0x00b6, 0x00b7, 0x00b8, 0x0105,
        0x015f, 0x00bb, 0x013d, 0x02dd, 0x013e, 0x017c, 0x0154, 0x00c1, 0x00c2, 0x0102, 0x00c4,
        0x0139, 0x0106, 0x00c7, 0x010c, 0x00c9, 0x0118, 0x00cb, 0x011a, 0x00cd, 0x00ce, 0x010e,
        0x0110, 0x0143, 0x0147, 0x00d3, 0x00d4, 0x0150, 0x00d6, 0x00d7, 0x0158, 0x016e, 0x00da,
        0x0170, 0x00dc, 0x00dd, 0x0162, 0x00df, 0x0155, 0x00e1, 0x00e2, 0x0103, 0x00e4, 0x013a,
        0x0107, 0x00e7, 0x010d, 0x00e9, 0x0119, 0x00eb, 0x011b, 0x00ed, 0x00ee, 0x010f, 0x0111,
        0x0144, 0x0148, 0x00f3, 0x00f4, 0x0151, 0x00f6, 0x00f7, 0x0159, 0x016f, 0x00fa, 0x0171,
        0x00fc, 0x00fd, 0x0163, 0x02d9,
    ];
    const CP1252: [u16; 32] = [
        0x20ac, 0, 0x201a, 0x0192, 0x201e, 0x2026, 0x2020, 0x2021, 0x02c6, 0x2030, 0x0160, 0x2039,
        0x0152, 0, 0x017d, 0, 0, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022, 0x2013, 0x2014, 0x02dc,
        0x2122, 0x0161, 0x203a, 0x0153, 0, 0x017e, 0x0178,
    ];
    let unit = if page == 1250 {
        CP1250[byte as usize - 128]
    } else if byte < 160 {
        CP1252[byte as usize - 128]
    } else {
        byte as u16
    };
    if unit == 0 {
        return Err("Undefined RTF code-page byte; not replaced".into());
    }
    Ok(char::from_u32(unit as u32).unwrap())
}

pub(super) fn decode(prefix: &[u8], boundary: &str) -> Decoded {
    decode_with_gaps(prefix, boundary, true)
}

#[cfg(test)]
pub(super) fn decode_v1(prefix: &[u8], boundary: &str) -> Decoded {
    decode_with_gaps(prefix, boundary, false)
}

fn decode_with_gaps(prefix: &[u8], boundary: &str, symbol_gaps: bool) -> Decoded {
    let mut out = Decoded {
        text: String::new(),
        through: 0,
        stop: boundary.into(),
        gaps: Vec::new(),
    };
    if !prefix.starts_with(b"{\\rtf1") {
        out.stop = "Readable RTF1 header unavailable".into();
        return out;
    }
    let mut state = State {
        skip: false,
        hidden: false,
        deleted: false,
        fonts: false,
        font: None,
        uc: 1,
        start: true,
    };
    let mut stack = Vec::new();
    let mut font_map: BTreeMap<i32, Font> = BTreeMap::new();
    let mut default_font = None;
    let mut default_page = 1252;
    let mut fallback = 0;
    let mut at = 0;
    let parsed = (|| -> Result<(), String> {
        while at < prefix.len() {
            let start = at;
            let byte = prefix[at];
            at += 1;
            if byte == b'{' || byte == b'}' {
                if fallback != 0 {
                    return Err("Unicode fallback crosses a group boundary".into());
                }
                if byte == b'{' {
                    if stack.len() >= MAX_DEPTH {
                        return Err("RTF nesting ceiling".into());
                    }
                    stack.push(state.clone());
                    state.start = true;
                } else {
                    state = stack.pop().ok_or("Unbalanced RTF group")?;
                    if stack.is_empty() {
                        out.through = at;
                        out.stop =
                            "Readable root group ended; original completeness remains unverified"
                                .into();
                        return Ok(());
                    }
                }
                out.through = at;
                continue;
            }
            if stack.is_empty() {
                return Err("Text outside RTF root".into());
            }
            if byte == b'\r' || byte == b'\n' {
                out.through = at;
                continue;
            }
            let mut t = if byte == b'\\' {
                token(prefix, &mut at)?
            } else {
                Token::Byte(byte)
            };
            if fallback != 0 {
                match t {
                    Token::Byte(_) | Token::Symbol(b'{' | b'}' | b'\\' | b'~' | b'-' | b'_') => {
                        fallback -= 1
                    }
                    _ => return Err("Unsupported Unicode fallback token".into()),
                }
                out.through = at;
                continue;
            }
            // Escaped literal bytes also use the selected font. Charset 2 is
            // explicitly single-byte SYMBOL: syntax/state remains readable,
            // but its glyph meaning is not an ANSI character or a guessed name.
            if symbol_gaps && let Token::Symbol(c @ (b'{' | b'}' | b'\\')) = t {
                t = Token::Byte(c);
            }
            let emit = match t {
                Token::Word(word, number) => {
                    if word == "bin" {
                        let n = number
                            .filter(|n| *n >= 0)
                            .ok_or("Invalid RTF binary count")?
                            as usize;
                        if !state.skip {
                            return Err("Binary content outside skipped destination".into());
                        }
                        at = at
                            .checked_add(n)
                            .filter(|end| *end <= prefix.len())
                            .ok_or("Binary payload crosses readable boundary")?;
                        None
                    } else if destination(word) {
                        if !state.start {
                            return Err("RTF destination outside group start".into());
                        }
                        if word == "fonttbl"
                            && !state.skip
                            && (!out.text.is_empty() || !out.gaps.is_empty())
                        {
                            return Err("RTF font table outside header".into());
                        }
                        state.fonts = word == "fonttbl" && !state.skip;
                        state.skip = true;
                        None
                    } else if state.fonts && matches!(word, "f" | "fcharset" | "cpg") {
                        let n = number.ok_or("Missing RTF font parameter")?;
                        if word == "f" {
                            if n < 0 || font_map.len() >= 256 {
                                return Err("RTF font ceiling/identity".into());
                            }
                            if font_map.contains_key(&n) {
                                return Err(
                                    "Duplicate RTF font definition; binding not guessed".into()
                                );
                            }
                            state.font = Some(n);
                            font_map.entry(n).or_default();
                        } else {
                            let f = font_map
                                .get_mut(&state.font.ok_or("RTF font properties without identity")?)
                                .ok_or("Missing RTF font")?;
                            if word == "cpg" {
                                if f.cpg.is_some() {
                                    return Err(
                                        "Duplicate RTF font code page; binding not guessed".into(),
                                    );
                                }
                                f.cpg = Some(n);
                            } else {
                                if f.charset.is_some() {
                                    return Err(
                                        "Duplicate RTF font charset; binding not guessed".into()
                                    );
                                }
                                f.charset = Some(n);
                            }
                        }
                        None
                    } else if state.skip {
                        None
                    } else {
                        match word {
                            "rtf" if number == Some(1) && stack.len() == 1 && start == 1 => None,
                            "ansi" => None,
                            "ansicpg" => {
                                if stack.len() != 1 || !out.text.is_empty() || !out.gaps.is_empty()
                                {
                                    return Err("RTF default code page outside header".into());
                                }
                                default_page = number
                                    .filter(|p| matches!(p, 1250 | 1252))
                                    .ok_or("Unsupported RTF default code page")?;
                                None
                            }
                            "deff" => {
                                if stack.len() != 1 || !out.text.is_empty() || !out.gaps.is_empty()
                                {
                                    return Err("RTF default font outside header".into());
                                }
                                default_font = Some(
                                    number
                                        .filter(|n| *n >= 0)
                                        .ok_or("Invalid RTF default font")?,
                                );
                                state.font = default_font;
                                None
                            }
                            "f" => {
                                state.font = Some(
                                    number
                                        .filter(|n| *n >= 0)
                                        .ok_or("Invalid RTF font identity")?,
                                );
                                None
                            }
                            "uc" => {
                                state.uc = number
                                    .filter(|n| (0..=16).contains(n))
                                    .ok_or("RTF Unicode fallback ceiling")?
                                    as usize;
                                None
                            }
                            "u" => {
                                let value = number
                                    .filter(|n| (-32768..=32767).contains(n))
                                    .ok_or("Invalid signed RTF Unicode unit")?
                                    as i16 as u16;
                                let c = char::from_u32(value as u32)
                                    .ok_or("Unsupported/split RTF UTF16 surrogate; not replaced")?;
                                fallback = state.uc;
                                Some(c)
                            }
                            "plain" => {
                                state.font = default_font;
                                state.hidden = false;
                                None
                            }
                            "v" => {
                                state.hidden = number != Some(0);
                                None
                            }
                            "deleted" => {
                                state.deleted = number != Some(0);
                                None
                            }
                            "par" | "line" | "row" | "page" | "sect" => Some('\n'),
                            "tab" | "cell" => Some('\t'),
                            "emdash" => Some('—'),
                            "endash" => Some('–'),
                            "bullet" => Some('•'),
                            "lquote" => Some('‘'),
                            "rquote" => Some('’'),
                            "ldblquote" => Some('“'),
                            "rdblquote" => Some('”'),
                            w if formatting(w) => None,
                            _ => return Err(format!("Unsupported visible RTF control: {word}")),
                        }
                    }
                }
                Token::Symbol(b'*') => {
                    if !state.start {
                        return Err("Ignorable destination marker outside group start".into());
                    }
                    state.skip = true;
                    state.fonts = false; // Unknown ignorable metadata cannot redefine a font.
                    out.through = at;
                    continue;
                }
                Token::Symbol(c @ (b'{' | b'}' | b'\\')) => Some(c as char),
                Token::Symbol(b'~') => Some('\u{00a0}'),
                Token::Symbol(b'_') => Some('\u{2011}'),
                Token::Symbol(b'-') => Some('\u{00ad}'),
                Token::Symbol(_) if state.skip => None,
                Token::Symbol(_) => return Err("Unsupported RTF control symbol".into()),
                Token::Byte(b) => {
                    if state.skip || state.hidden || state.deleted {
                        None
                    } else {
                        if b < 32 && b != b'\t' {
                            return Err("Unexpected RTF text control byte".into());
                        }
                        if symbol_gaps
                            && let Some(id) = state.font
                            && font_map
                                .get(&id)
                                .is_some_and(|f| f.charset == Some(2) && f.cpg.is_none())
                        {
                            if out.gaps.len() >= MAX_GAPS {
                                return Err("RTF unsupported-glyph gap ceiling".into());
                            }
                            out.gaps.push(GlyphGap {
                                input_start: start,
                                input_end: at,
                                output_utf8_offset: out.text.len(),
                                font_id: id,
                                encoded_byte: b,
                            });
                            state.start = false;
                            out.through = at;
                            continue;
                        }
                        let page = match state.font {
                            None => Some(default_page),
                            Some(id) => font_map
                                .get(&id)
                                .map(|f| {
                                    f.cpg.or(match f.charset {
                                        None => Some(default_page),
                                        Some(0) => Some(1252),
                                        Some(238) => Some(1250),
                                        _ => None,
                                    })
                                })
                                .unwrap_or(None),
                        };
                        Some(byte_character(b, page)?)
                    }
                }
            };
            state.start = false;
            if !state.skip
                && !state.hidden
                && !state.deleted
                && let Some(c) = emit
            {
                if out.text.len() + c.len_utf8() > MAX_TEXT {
                    return Err("RTF text output ceiling".into());
                }
                out.text.push(c);
            }
            out.through = at;
        }
        if fallback != 0 {
            return Err("Unicode fallback crosses readable boundary".into());
        }
        Ok(())
    })();
    if let Err(error) = parsed {
        out.stop = error;
    }
    out
}
