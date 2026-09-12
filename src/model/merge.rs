//! Combining several airport records that describe the same airport.
//!
//! Scenery packages routinely split one airport across several BGL files, or
//! repeat the identifier inside one file: a base layout in one record and
//! jetways, paint or an updated apron in another. MSFS applies them in order,
//! so the converter has to as well, or a package looks half-built.
//!
//! Merging is additive. The taxi network is the awkward part, because both
//! airports number their nodes from zero; the incoming indices are rebased so
//! that paths keep pointing at their own nodes.

use super::*;

impl Airport {
    /// Fold `other` into `self`, rebasing its taxi node indices.
    pub fn merge_from(&mut self, other: Airport) {
        // Scalar fields: keep what we have, take anything we are missing.
        if self.name.is_empty() {
            self.name = other.name;
        }
        if self.city.is_none() {
            self.city = other.city;
        }
        if self.state.is_none() {
            self.state = other.state;
        }
        if self.country.is_none() {
            self.country = other.country;
        }
        if self.region.is_none() {
            self.region = other.region;
        }
        if self.tower.is_none() {
            self.tower = other.tower;
        }
        if !self.datum.is_valid() {
            self.datum = other.datum;
            self.elevation_m = other.elevation_m;
        }
        self.closed |= other.closed;
        self.flatten |= other.flatten;
        self.replaces_stock |= other.replaces_stock;

        let offset = self.taxi_nodes.iter().map(|n| n.index + 1).max().unwrap_or(0);

        self.taxi_nodes.extend(other.taxi_nodes.into_iter().map(|mut n| {
            n.index += offset;
            n
        }));
        self.taxi_paths.extend(other.taxi_paths.into_iter().map(|mut p| {
            p.start += offset;
            p.end += offset;
            p
        }));
        self.parkings.extend(other.parkings.into_iter().map(|mut p| {
            p.index += offset;
            p
        }));

        self.runways.extend(other.runways);
        self.helipads.extend(other.helipads);
        self.starts.extend(other.starts);
        self.aprons.extend(other.aprons);
        self.apron_edge_lights.extend(other.apron_edge_lights);
        self.painted_lines.extend(other.painted_lines);
        self.signs.extend(other.signs);
        self.windsocks.extend(other.windsocks);
        self.beacons.extend(other.beacons);
        self.warnings.extend(other.warnings);

        // Radios are frequently repeated verbatim between files.
        for com in other.coms {
            if !self
                .coms
                .iter()
                .any(|c| c.kind == com.kind && c.freq_khz == com.freq_khz)
            {
                self.coms.push(com);
            }
        }
    }
}

/// Merge a list of airports so each identifier appears once, preserving order.
pub fn merge_by_ident(airports: Vec<Airport>) -> Vec<Airport> {
    let mut order: Vec<String> = Vec::new();
    let mut by_ident: std::collections::HashMap<String, Airport> = std::collections::HashMap::new();
    for ap in airports {
        let key = ap.icao.to_ascii_uppercase();
        match by_ident.get_mut(&key) {
            Some(existing) => existing.merge_from(ap),
            None => {
                order.push(key.clone());
                by_ident.insert(key, ap);
            }
        }
    }
    order.into_iter().filter_map(|k| by_ident.remove(&k)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::LatLon;

    fn node(index: usize) -> TaxiNode {
        TaxiNode {
            index,
            pos: LatLon::new(25.0, 55.0),
            ..Default::default()
        }
    }

    fn airport(icao: &str, nodes: usize) -> Airport {
        Airport {
            icao: icao.to_string(),
            datum: LatLon::new(25.0, 55.0),
            taxi_nodes: (0..nodes).map(node).collect(),
            taxi_paths: if nodes >= 2 {
                vec![TaxiPath {
                    start: 0,
                    end: nodes - 1,
                    kind: PathKind::Taxi,
                    ..Default::default()
                }]
            } else {
                Vec::new()
            },
            ..Default::default()
        }
    }

    #[test]
    fn merging_rebases_node_indices_so_paths_stay_connected() {
        let mut a = airport("OMDB", 3);
        let b = airport("OMDB", 2);
        a.merge_from(b);

        assert_eq!(a.taxi_nodes.len(), 5);
        let indices: Vec<usize> = a.taxi_nodes.iter().map(|n| n.index).collect();
        assert_eq!(indices, vec![0, 1, 2, 3, 4], "indices must stay unique");

        // The second airport's path pointed at its own nodes 0 and 1; after
        // rebasing it must point at 3 and 4, not back into the first airport.
        assert_eq!((a.taxi_paths[0].start, a.taxi_paths[0].end), (0, 2));
        assert_eq!((a.taxi_paths[1].start, a.taxi_paths[1].end), (3, 4));
        assert!(a.node(4).is_some());
    }

    #[test]
    fn merging_rebases_parking_indices_too() {
        let mut a = airport("OMDB", 2);
        let mut b = airport("OMDB", 2);
        b.parkings.push(Parking {
            index: 1,
            name: "A1".into(),
            ..Default::default()
        });
        a.merge_from(b);
        assert_eq!(a.parkings[0].index, 3);
        assert!(a.node(3).is_some());
    }

    #[test]
    fn merging_fills_missing_scalars_but_does_not_overwrite() {
        let mut a = Airport {
            icao: "OMDB".into(),
            name: "Dubai".into(),
            ..Default::default()
        };
        let b = Airport {
            icao: "OMDB".into(),
            name: "Something Else".into(),
            city: Some("Dubai".into()),
            closed: true,
            ..Default::default()
        };
        a.merge_from(b);
        assert_eq!(a.name, "Dubai", "an existing name wins");
        assert_eq!(a.city.as_deref(), Some("Dubai"), "a missing field is filled");
        assert!(a.closed, "flags accumulate");
    }

    #[test]
    fn duplicate_frequencies_are_not_repeated() {
        let com = Com {
            kind: ComKind::Tower,
            freq_khz: 118_400,
            name: "TWR".into(),
        };
        let mut a = Airport {
            coms: vec![com.clone()],
            ..Default::default()
        };
        let b = Airport {
            coms: vec![
                com,
                Com {
                    kind: ComKind::Ground,
                    freq_khz: 121_800,
                    name: "GND".into(),
                },
            ],
            ..Default::default()
        };
        a.merge_from(b);
        assert_eq!(a.coms.len(), 2);
    }

    #[test]
    fn merge_by_ident_groups_and_preserves_order() {
        let list = vec![airport("OMDB", 2), airport("OMDW", 1), airport("omdb", 2)];
        let merged = merge_by_ident(list);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].icao.to_ascii_uppercase(), "OMDB");
        assert_eq!(merged[1].icao.to_ascii_uppercase(), "OMDW");
        assert_eq!(merged[0].taxi_nodes.len(), 4, "case-insensitive idents merge");
    }

    #[test]
    fn merging_into_an_empty_airport_takes_the_datum() {
        let mut a = Airport {
            icao: "OMDB".into(),
            ..Default::default()
        };
        a.datum = LatLon::new(f64::NAN, f64::NAN);
        let b = airport("OMDB", 1);
        a.merge_from(b);
        assert!(a.datum.is_valid());
    }
}
