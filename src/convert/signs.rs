//! Taxiway sign text: MSFS label syntax to X-Plane sign syntax.
//!
//! MSFS: a lower-case letter picks the panel colour for what follows
//! (`l` location, `d`/`i`/`u` direction or information, `m`/`r` mandatory),
//! square brackets draw borders, and punctuation draws arrows.
//! X-Plane: `{@L}`, `{@Y}`, `{@R}` pick the panel colour, `{^l}` and friends draw
//! arrows, and a colour change starts a new panel. Borders are drawn
//! automatically, so MSFS brackets are simply dropped.

use crate::model::Airport;
use crate::xplane::apt::{AptAirport, Sign};

use super::Report;

/// Translate a label, or `None` when it uses a glyph X-Plane cannot draw.
pub fn translate_label(label: &str) -> Option<String> {
    let mut out = String::new();
    let mut colour: Option<&str> = None;
    let mut glyphs = 0usize;
    for c in label.chars() {
        let token: Option<&str> = match c {
            'l' => {
                colour = Some("{@L}");
                None
            }
            'd' | 'i' | 'u' => {
                colour = Some("{@Y}");
                None
            }
            'm' | 'r' => {
                colour = Some("{@R}");
                None
            }
            '[' | ']' => None,
            '>' => Some("{^r}"),
            '<' => Some("{^l}"),
            '^' => Some("{^u}"),
            'v' => Some("{^d}"),
            '\'' => Some("{^ru}"),
            '`' => Some("{^lu}"),
            '/' => Some("{^ld}"),
            '\\' => Some("{^rd}"),
            '_' | ' ' | '|' => Some("{_}"),
            '-' => Some("-"),
            c if c.is_ascii_uppercase() || c.is_ascii_digit() => {
                let pending = colour.take();
                if let Some(p) = pending {
                    if !out.ends_with(p) {
                        out.push_str(p);
                    }
                }
                out.push(c);
                glyphs += 1;
                continue;
            }
            _ => return None,
        };
        if let Some(t) = token {
            if let Some(p) = colour.take() {
                if !out.ends_with(p) {
                    out.push_str(p);
                }
            }
            out.push_str(t);
            glyphs += 1;
        }
    }
    if glyphs == 0 {
        return None;
    }
    // A label must open with a colour; MSFS defaults to a direction panel.
    if !out.starts_with("{@") {
        out.insert_str(0, "{@Y}");
    }
    Some(out)
}

pub fn build(ap: &Airport, out: &mut AptAirport, report: &mut Report) {
    for s in &ap.signs {
        match translate_label(&s.label) {
            Some(text) => {
                out.signs.push(Sign {
                    lat: s.pos.lat,
                    lon: s.pos.lon,
                    heading: s.heading,
                    // X-Plane has three panel heights; MSFS five.
                    size: match s.size {
                        0..=2 => 1,
                        3 => 2,
                        _ => 3,
                    },
                    text,
                });
                report.converted("taxiway signs", 1);
            }
            None => report.dropped("taxiway signs with untranslatable text", 1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_sign() {
        assert_eq!(translate_label("l[G]").as_deref(), Some("{@L}G"));
    }

    #[test]
    fn combined_sign_with_arrow_and_runway() {
        assert_eq!(
            translate_label("l[G]d[F\\]m[11R-29L]").as_deref(),
            Some("{@L}G{@Y}F{^rd}{@R}11R-29L")
        );
    }

    #[test]
    fn arrows_and_spaces() {
        assert_eq!(translate_label("d[<A_B>]").as_deref(), Some("{@Y}{^l}A{_}B{^r}"));
    }

    #[test]
    fn unknown_glyphs_and_empty_labels_are_rejected() {
        assert!(translate_label("d[A*]").is_none());
        assert!(translate_label("l[]").is_none());
    }

    #[test]
    fn bare_text_gets_a_default_colour() {
        assert_eq!(translate_label("A5").as_deref(), Some("{@Y}A5"));
    }
}
