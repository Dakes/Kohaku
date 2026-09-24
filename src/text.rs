//! Character rules for short user-entered text (projects: Project fields; change
//! projects D6), shared with later title and close-reason validation.

/// Whether `c` may hide or reorder text: category Cc, the line and paragraph
/// separators, and the invisible or bidirectional format characters.
fn is_hiding(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{2028}'
                | '\u{2029}'
                | '\u{00AD}'
                | '\u{061C}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206F}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{E0000}'..='\u{E007F}'
        )
}

/// Single line: no character that [`is_hiding`] refuses, line feeds included.
pub fn is_single_line(text: &str) -> bool {
    !text.chars().any(is_hiding)
}

/// Like [`is_single_line`], but line feeds are allowed.
pub fn is_multi_line(text: &str) -> bool {
    !text.chars().any(|c| c != '\n' && is_hiding(c))
}

/// CR LF and lone CR as LF.
pub fn normalize_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTED: &[char] = &[
        '\u{2028}',
        '\u{2029}',
        '\u{00AD}',
        '\u{061C}',
        '\u{180E}',
        '\u{200B}',
        '\u{200C}',
        '\u{200D}',
        '\u{200E}',
        '\u{200F}',
        '\u{202A}',
        '\u{202B}',
        '\u{202C}',
        '\u{202D}',
        '\u{202E}',
        '\u{2060}',
        '\u{2061}',
        '\u{2062}',
        '\u{2063}',
        '\u{2064}',
        '\u{2066}',
        '\u{2067}',
        '\u{2068}',
        '\u{2069}',
        '\u{206A}',
        '\u{206F}',
        '\u{FEFF}',
        '\u{FFF9}',
        '\u{FFFA}',
        '\u{FFFB}',
        '\u{E0000}',
        '\u{E0001}',
        '\u{E0041}',
        '\u{E007F}',
    ];

    #[test]
    fn every_listed_character_is_refused() {
        for &c in LISTED {
            let text = format!("a{c}b");
            assert!(!is_single_line(&text), "U+{:04X}", u32::from(c));
            assert!(!is_multi_line(&text), "U+{:04X}", u32::from(c));
        }
    }

    #[test]
    fn control_characters_are_refused() {
        for c in (0u32..0x20).chain(0x7f..0xa0).filter_map(char::from_u32) {
            let text = format!("a{c}b");
            assert!(!is_single_line(&text), "U+{:04X}", u32::from(c));
            assert_eq!(is_multi_line(&text), c == '\n', "U+{:04X}", u32::from(c));
        }
    }

    #[test]
    fn ordinary_text_passes() {
        for text in [
            "Demo app",
            "Émile's app — 日本語",
            "a\u{00A0}b",
            "x\u{2065}y",
            "",
        ] {
            assert!(is_single_line(text), "{text:?}");
        }
        assert!(is_multi_line("one\ntwo"));
        // Neighbours of the listed ranges stay allowed.
        for c in ['\u{2027}', '\u{202F}', '\u{2070}', '\u{FFFC}', '\u{E0080}'] {
            assert!(is_single_line(&c.to_string()), "U+{:04X}", u32::from(c));
        }
    }

    #[test]
    fn line_endings() {
        assert_eq!(normalize_line_endings("a\r\nb\rc\nd"), "a\nb\nc\nd");
        assert_eq!(normalize_line_endings("\r\r\n"), "\n\n");
    }
}
