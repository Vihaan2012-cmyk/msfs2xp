//! Decoders for the packed numeric formats used inside BGL records.

/// Longitude from the BGL 32-bit fixed-point encoding.
///
/// The full 360 degrees are mapped onto `3 * 2^28` units.
pub fn lon_from_u32(v: u32) -> f64 {
    (v as f64) * (360.0 / (3.0 * 0x1000_0000 as f64)) - 180.0
}

/// Latitude from the BGL 32-bit fixed-point encoding (north-down).
pub fn lat_from_u32(v: u32) -> f64 {
    90.0 - (v as f64) * (180.0 / (2.0 * 0x1000_0000 as f64))
}

/// Altitude in metres from the BGL millimetre integer.
pub fn alt_from_i32(v: i32) -> f64 {
    v as f64 / 1000.0
}

/// Encode a longitude back into the BGL representation (used by tests and tools).
pub fn lon_to_u32(lon: f64) -> u32 {
    (((lon + 180.0) / (360.0 / (3.0 * 0x1000_0000 as f64))).round()).clamp(0.0, u32::MAX as f64) as u32
}

/// Encode a latitude back into the BGL representation.
pub fn lat_to_u32(lat: f64) -> u32 {
    (((90.0 - lat) / (180.0 / (2.0 * 0x1000_0000 as f64))).round()).clamp(0.0, u32::MAX as f64) as u32
}

fn decode_base38(value: u64, max_chars: usize) -> String {
    // Digits 2..=11 encode '0'..'9'; 12..=37 encode 'A'..'Z'. 0 and 1 terminate.
    let mut coded = [0u64; 8];
    let mut idx = 0usize;
    let mut value = value;
    if value == 0 {
        return String::new();
    }
    if value > 37 {
        while value > 37 {
            if idx >= coded.len() {
                return String::new();
            }
            let c = value % 38;
            coded[idx] = c;
            idx += 1;
            value = (value - c) / 38;
            if value < 38 {
                if idx >= coded.len() {
                    return String::new();
                }
                coded[idx] = value;
                idx += 1;
                break;
            }
        }
    } else {
        coded[idx] = value;
        idx += 1;
    }
    let _ = idx;
    let mut out = String::new();
    for &c in coded.iter().take(max_chars) {
        if c == 0 {
            break;
        }
        if (2..12).contains(&c) {
            out.insert(0, (b'0' + (c as u8 - 2)) as char);
        } else if c >= 12 {
            out.insert(0, (b'A' + (c as u8 - 12)) as char);
        } else {
            break;
        }
    }
    out
}

/// Decode a 32-bit packed identifier (up to five characters).
///
/// Most BGL identifiers are stored shifted left by five bits; navaid records
/// that store the raw value pass `shifted = false`.
pub fn icao_from_u32(v: u32, shifted: bool) -> String {
    let value = if shifted { (v >> 5) as u64 } else { v as u64 };
    decode_base38(value, 5)
}

/// Decode the 64-bit packed identifier introduced in MSFS 2024 (up to eight characters).
pub fn icao_from_u64(v: u64, shifted: bool) -> String {
    let value = if shifted { v >> 6 } else { v };
    decode_base38(value, 8)
}

/// Encode an identifier the way BGL stores it (test helper, also used by `inspect`).
pub fn icao_to_u32(ident: &str, shifted: bool) -> u32 {
    let mut value: u64 = 0;
    for ch in ident.chars() {
        let c = match ch {
            '0'..='9' => ch as u64 - '0' as u64 + 2,
            'A'..='Z' => ch as u64 - 'A' as u64 + 12,
            'a'..='z' => ch as u64 - 'a' as u64 + 12,
            _ => continue,
        };
        value = value * 38 + c;
    }
    let v = if shifted { value << 5 } else { value };
    v as u32
}

/// Runway designator letter for the BGL designator code.
pub fn designator_str(designator: u8) -> &'static str {
    match designator {
        1 => "L",
        2 => "R",
        3 => "C",
        4 => "W",
        5 => "A",
        6 => "B",
        _ => "",
    }
}

/// Full runway end name, e.g. `09L`, `27`, `NE`.
pub fn runway_name(number: u8, designator: u8) -> String {
    let base = match number {
        0..=9 => format!("0{number}"),
        10..=36 => format!("{number}"),
        37 => "N".to_string(),
        38 => "NE".to_string(),
        39 => "E".to_string(),
        40 => "SE".to_string(),
        41 => "S".to_string(),
        42 => "SW".to_string(),
        43 => "W".to_string(),
        44 => "NW".to_string(),
        _ => format!("{number}"),
    };
    if number > 36 {
        base
    } else {
        format!("{base}{}", designator_str(designator))
    }
}

/// Convert the BGL magnetic variation convention to "east negative, west positive" degrees.
pub fn magvar_adjust(raw: f32) -> f32 {
    -(if raw > 180.0 { raw - 360.0 } else { raw })
}

/// Normalise a heading into `[0, 360)`.
pub fn normalize_heading(h: f64) -> f64 {
    let mut h = h % 360.0;
    if h < 0.0 {
        h += 360.0;
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lon_lat_roundtrip() {
        for &(lat, lon) in &[(40.6398, -73.7789), (-33.9461, 151.1772), (0.0, 0.0), (78.2, 15.5)] {
            let rlat = lat_from_u32(lat_to_u32(lat));
            let rlon = lon_from_u32(lon_to_u32(lon));
            assert!((rlat - lat).abs() < 1e-6, "lat {lat} -> {rlat}");
            assert!((rlon - lon).abs() < 1e-6, "lon {lon} -> {rlon}");
        }
    }

    #[test]
    fn lon_zero_is_minus_180() {
        assert_eq!(lon_from_u32(0), -180.0);
        assert_eq!(lat_from_u32(0), 90.0);
    }

    #[test]
    fn altitude_is_millimetres() {
        assert_eq!(alt_from_i32(3_500), 3.5);
        assert_eq!(alt_from_i32(-1_000), -1.0);
    }

    #[test]
    fn icao_roundtrip() {
        for ident in ["KJFK", "EGLL", "LFPG", "0S9", "X01", "VQPR"] {
            assert_eq!(icao_from_u32(icao_to_u32(ident, true), true), ident);
        }
    }

    #[test]
    fn icao_empty_for_zero() {
        assert_eq!(icao_from_u32(0, true), "");
    }

    #[test]
    fn runway_names() {
        assert_eq!(runway_name(9, 0), "09");
        assert_eq!(runway_name(27, 2), "27R");
        assert_eq!(runway_name(4, 1), "04L");
        assert_eq!(runway_name(13, 3), "13C");
        assert_eq!(runway_name(37, 0), "N");
        assert_eq!(runway_name(40, 0), "SE");
    }

    #[test]
    fn magvar_signs() {
        assert_eq!(magvar_adjust(13.0), -13.0);
        assert_eq!(magvar_adjust(350.0), 10.0);
    }

    #[test]
    fn heading_normalises() {
        assert_eq!(normalize_heading(-10.0), 350.0);
        assert_eq!(normalize_heading(370.0), 10.0);
        assert_eq!(normalize_heading(180.0), 180.0);
    }
}
