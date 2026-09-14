//! Code tables mapping MSFS concepts onto apt.dat values.
//!
//! These are judgement calls as much as lookups: MSFS has two dozen surface
//! materials, X-Plane has a handful, and several MSFS lighting systems have no
//! exact X-Plane twin. Each choice here picks the closest visual result.

use crate::model::{AlsSystem, ComKind, LightLevel, RunwayMarkings, Side, Surface, VasiKind};
use crate::xplane::apt::{row, surface};

/// Nearest X-Plane surface code.
pub fn surface_code(s: &Surface) -> u8 {
    match s {
        Surface::Asphalt | Surface::Bituminous | Surface::Tarmac | Surface::Macadam | Surface::OilTreated => {
            surface::ASPHALT
        }
        Surface::Concrete | Surface::Cement | Surface::Brick | Surface::SteelMats | Surface::Planks => {
            surface::CONCRETE
        }
        Surface::Grass => surface::GRASS,
        Surface::Dirt | Surface::Clay | Surface::Sand | Surface::Shale | Surface::Coral => surface::DIRT,
        Surface::Gravel => surface::GRAVEL,
        Surface::Snow | Surface::Ice => surface::SNOW_ICE,
        Surface::Water => surface::WATER,
        Surface::Transparent => surface::TRANSPARENT,
        Surface::Unknown => surface::ASPHALT,
    }
}

/// X-Plane 12 surface code with a shade chosen from the material texture's
/// mean brightness (0..=255): the asphalt variants 20/24/27/31/35 run from
/// light to near black and the concrete variants 50/53/55 from new to dark.
/// Without a brightness the plain X-Plane 11 codes stand.
pub fn shaded_surface_code(s: &Surface, brightness: Option<u8>) -> u8 {
    let base = surface_code(s);
    match (base, brightness) {
        (surface::ASPHALT, Some(b)) => match b {
            115.. => 20,
            90..=114 => 24,
            78..=89 => 27,
            60..=77 => 31,
            _ => 35,
        },
        (surface::CONCRETE, Some(b)) => match b {
            135.. => 50,
            100..=134 => 53,
            _ => 55,
        },
        _ => base,
    }
}

/// Runway marking style for one end.
///
/// Precision beats non-precision beats visual; MSFS's "alternate" flags mark the
/// ICAO layout, which X-Plane calls EASA.
pub fn marking_code(m: &RunwayMarkings) -> u8 {
    if m.precision {
        if m.alternate {
            7
        } else {
            3
        }
    } else if m.touchdown || m.fixed_distance {
        if m.alternate {
            6
        } else {
            2
        }
    } else if m.threshold || m.ident || m.edges || m.dashes {
        1
    } else {
        0
    }
}

/// Approach lighting system code.
pub fn als_code(a: AlsSystem) -> u8 {
    match a {
        AlsSystem::None => 0,
        AlsSystem::Alsf1 => 1,
        AlsSystem::Alsf2 => 2,
        AlsSystem::Calvert => 3,
        AlsSystem::Calvert2 => 4,
        AlsSystem::Ssalr => 5,
        AlsSystem::Ssalf => 6,
        // X-Plane has one "SALS" type for the whole simplified family.
        AlsSystem::Sals | AlsSystem::Ssals | AlsSystem::Salsf => 7,
        AlsSystem::Malsr => 8,
        AlsSystem::Malsf => 9,
        AlsSystem::Mals => 10,
        AlsSystem::Odals => 11,
        AlsSystem::Rail => 12,
    }
}

pub fn edge_light_code(l: LightLevel) -> u8 {
    match l {
        LightLevel::None => 0,
        LightLevel::Low => 1,
        LightLevel::Medium => 2,
        LightLevel::High => 3,
    }
}

/// The 8.33 kHz frequency row for a radio service, if X-Plane has one.
pub fn com_code(k: ComKind) -> Option<u16> {
    Some(match k {
        ComKind::Atis | ComKind::Awos | ComKind::Asos => row::FREQ_AWOS,
        ComKind::Unicom | ComKind::Ctaf | ComKind::Multicom => row::FREQ_CTAF,
        ComKind::Clearance | ComKind::ClearancePreTaxi | ComKind::RemoteClearance => row::FREQ_CLEARANCE,
        ComKind::Ground => row::FREQ_GROUND,
        ComKind::Tower => row::FREQ_TOWER,
        ComKind::Approach => row::FREQ_APPROACH,
        ComKind::Departure => row::FREQ_DEPARTURE,
        ComKind::Center | ComKind::Fss | ComKind::Unknown => return None,
    })
}

/// Light object type for a glideslope indicator on a given side.
pub fn vasi_code(k: VasiKind, side: Side) -> Option<u8> {
    Some(match (k, side) {
        (VasiKind::Papi4 | VasiKind::Papi2, Side::Left) => 2,
        (VasiKind::Papi4 | VasiKind::Papi2, Side::Right) => 3,
        (VasiKind::Vasi | VasiKind::PulsatingVasi | VasiKind::TVasi, _) => 1,
        (VasiKind::TriColor, _) => 5,
        (VasiKind::Apap | VasiKind::Ball, Side::Left) => 7,
        (VasiKind::Apap | VasiKind::Ball, Side::Right) => 8,
        (VasiKind::None, _) => return None,
    })
}

/// ICAO aircraft size class from wingspan in metres.
pub fn width_class_from_span(span_m: f32) -> char {
    match span_m {
        s if s < 15.0 => 'A',
        s if s < 24.0 => 'B',
        s if s < 36.0 => 'C',
        s if s < 52.0 => 'D',
        s if s < 65.0 => 'E',
        _ => 'F',
    }
}

/// ICAO taxiway class from pavement width in metres (Annex 14 minimum widths).
pub fn width_class_from_taxiway(width_m: f32) -> char {
    match width_m {
        w if w < 10.5 => 'A',
        w if w < 15.0 => 'B',
        w if w < 18.0 => 'C',
        w if w < 23.0 => 'D',
        w if w < 25.0 => 'E',
        _ => 'F',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surfaces_collapse_to_the_nearest_texture() {
        assert_eq!(surface_code(&Surface::Tarmac), surface::ASPHALT);
        assert_eq!(surface_code(&Surface::Cement), surface::CONCRETE);
        assert_eq!(surface_code(&Surface::Sand), surface::DIRT);
        assert_eq!(surface_code(&Surface::Water), surface::WATER);
        assert_eq!(surface_code(&Surface::Transparent), surface::TRANSPARENT);
        // iniBuilds' Dubai: taxiway asphalt averages 75, concrete tiles 141,
        // Asobo's cement 107.
        assert_eq!(shaded_surface_code(&Surface::Asphalt, Some(75)), 31);
        assert_eq!(shaded_surface_code(&Surface::Asphalt, Some(40)), 35);
        assert_eq!(shaded_surface_code(&Surface::Asphalt, Some(120)), 20);
        assert_eq!(shaded_surface_code(&Surface::Concrete, Some(141)), 50);
        assert_eq!(shaded_surface_code(&Surface::Cement, Some(107)), 53);
        assert_eq!(shaded_surface_code(&Surface::Concrete, Some(80)), 55);
        assert_eq!(shaded_surface_code(&Surface::Asphalt, None), surface::ASPHALT);
        assert_eq!(shaded_surface_code(&Surface::Dirt, Some(200)), surface::DIRT);
    }

    #[test]
    fn markings_pick_the_most_capable_style() {
        let mut m = RunwayMarkings {
            threshold: true,
            ..Default::default()
        };
        assert_eq!(marking_code(&m), 1);
        m.touchdown = true;
        assert_eq!(marking_code(&m), 2);
        m.precision = true;
        assert_eq!(marking_code(&m), 3);
        m.alternate = true;
        assert_eq!(marking_code(&m), 7);
        assert_eq!(marking_code(&RunwayMarkings::default()), 0);
    }

    #[test]
    fn approach_lights_and_radios_map() {
        assert_eq!(als_code(AlsSystem::Alsf2), 2);
        assert_eq!(als_code(AlsSystem::Ssals), 7);
        assert_eq!(als_code(AlsSystem::Rail), 12);
        assert_eq!(com_code(ComKind::Tower), Some(row::FREQ_TOWER));
        assert_eq!(com_code(ComKind::Asos), Some(row::FREQ_AWOS));
        assert_eq!(com_code(ComKind::Center), None);
    }

    #[test]
    fn papi_side_decides_handedness() {
        assert_eq!(vasi_code(VasiKind::Papi4, Side::Left), Some(2));
        assert_eq!(vasi_code(VasiKind::Papi4, Side::Right), Some(3));
        assert_eq!(vasi_code(VasiKind::None, Side::Left), None);
    }

    #[test]
    fn size_classes() {
        assert_eq!(width_class_from_span(11.0), 'A');
        assert_eq!(width_class_from_span(35.8), 'C'); // A320
        assert_eq!(width_class_from_span(64.8), 'E'); // 777-300ER
        assert_eq!(width_class_from_span(79.8), 'F'); // A380
        assert_eq!(width_class_from_taxiway(23.0), 'E');
        assert_eq!(width_class_from_taxiway(7.5), 'A');
    }
}
