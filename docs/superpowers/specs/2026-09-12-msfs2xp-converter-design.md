# msfs2xp — MSFS 2020/2024 airport → X-Plane 12 converter (design)

Date: 2026-09-12. Status: approved for implementation (autonomous session; decisions recorded here).

## 1. Goal

A Rust CLI, `msfs2xp`, that converts airports from Microsoft Flight Simulator 2020 and
2024 packages into X-Plane 12 custom-scenery airports (`apt.dat` version 1200).
Inputs must cover everything a user realistically has on disk:

| Source | Format | Notes |
|---|---|---|
| Asobo/Microsoft hand-crafted airports (2020, Official/OneStore) | compiled BGL | on this machine, used as real-world fixtures |
| Third-party payware/freeware (iniBuilds, FlyTampa, …) for 2020 and 2024 | compiled BGL in `Community/<pkg>/…` | primary target |
| Marketplace packages in Official/OneStore | BGL, sometimes DRM-encrypted | detect encryption, report clearly |
| `fs-base-genericairports` | 4 824 BGLs holding ~37 000 stock airports | batch / `--icao` filter |
| SDK source projects | `FSData` XML | developers |

Out of scope: MSFS 2024 streamed stock airports (`.fsarchive`), navdata (approaches,
SIDs/STARs, ILS), 3D models, textures, terraforming, jetway animation.

## 2. Reference facts (verified this session)

* BGL container: header 0x38 bytes (magic `0x19920201`, `0x08051803`), N section
  headers of 0x14 bytes, subsection headers of 0x10 bytes (`id, recordCount,
  offset, size`). Airport section type `0x03`; airport record id `0x003C` in every
  sim. Every record starts `u16 id, u32 size` so unknown records are skippable.
* Coordinates: `lon = u32 * 360/(3·2^28) − 180`, `lat = 90 − u32 * 180/(2·2^28)`,
  altitude `i32 / 1000` m. ICAO idents are base-38 packed u32 shifted by 5 bits.
* Record layouts for FSX/P3D/MSFS 2020 (airport, runway + subrecords, taxi point,
  taxi path, parking, apron, helipad, start, com, jetway, taxi name, delete) taken
  from atools (GPL-3, used only as a format reference; code is original).
* MSFS-only records with no public layout (painted line `0x00CF`, taxiway sign
  `0x00D9`, apron edge lights `0x0031`, blast/boundary fence `0x38/0x39`, light
  support `0x57`, jetway `0x00DE`, hatched area `0x00D8`): reverse-engineered
  during implementation from the local Asobo BGLs; parsed best-effort and
  fully guarded.
* MSFS 2024 SDK BGLs: same container; new enum values (`taxipath ROAD=7,
  PAINTEDLINE=8`, `start TRACK=4`, parking type `0x10`). Layout differences are
  handled by size-driven variant detection (§5.3).
* X-Plane 12 apt.dat 1200 row codes and enumerations taken from the Laminar
  spec and WED `AptDefs.h` (surface 1–15, 20–38, 50–57; markings 0–7; approach
  lights 0–12; line codes 1–9, 20–22, 51–59; light codes 101–106; rows 1, 16, 17,
  14, 18, 19, 20, 21, 100–102, 110–116, 120, 130, 1050–1056, 1200–1206,
  1300–1302, 1400–1401, 99).
* MSFS taxiway sign grammar: `l|d|m|i|r|u` type prefix, `[ ]` borders, arrows
  `<`, `>`, `^`, `v`, apostrophe (up-right), backquote (up-left), `/` (down-left),
  `\` (down-right), `_` space, `-`, `|`. X-Plane grammar: `{@L}{@Y}{@R}{@B}`,
  `{^l}{^r}{^u}{^d}{^lu}{^ru}{^ld}{^rd}`, `{@@}` back face, `{_}` separator.

## 3. Architecture

Single crate, library + thin binary. Modules, each independently testable:

```
src/
  main.rs            clap CLI → lib
  cli.rs             subcommands: convert | inspect | list | preview | validate
  package/           discovery: single file, package dir, Community dir; manifest.json,
                     layout.json; sim detection; encrypted-file detection
  bgl/
    reader.rs        bounds-checked little-endian cursor (never panics)
    file.rs          header/sections/subsections → iterator of airport records
    variant.rs       layout variant detection (Fsx | P3d | Msfs2020 | Msfs2024 | Unknown)
    records/*.rs     one parser per record type → bgl::raw structs
    inspect.rs       human-readable record tree + hexdump of unknown records
  xml/               FSData XML → model (roxmltree, lenient)
  model/             sim-neutral Airport domain model (WGS84, metres, enums)
  geo/               WGS84 destination/bearing (Vincenty), local tangent plane,
                     polygon offsetting, boolean union (geo crate) with fallback
  convert/           model → xplane::Apt (one file per concern: runways, helipads,
                     pavement, lines, network, ramps, coms, signs, misc)
  xplane/
    apt.rs           typed row structs + writer
    validate.rs      strict re-parser used by tests and `validate`
  preview/           standalone HTML/SVG renderer of an apt.dat
```

Data flow: `package::discover → bgl::file / xml → model::Airport (merged per ICAO
within a package) → convert → xplane::Apt → apt.dat + preview + JSON`.

## 4. CLI

```
msfs2xp convert <INPUT>... -o <OUT> [--icao KJFK,EGLL] [--merge] [--sim auto|2020|2024]
        [--no-union] [--no-network] [--no-lines] [--preview] [--json] [--jobs N] [-v]
msfs2xp list <INPUT>...            # airports found, with counts and detected variant
msfs2xp inspect <file.bgl> [--icao X] [--hex]   # record tree for reverse engineering
msfs2xp preview <apt.dat> -o page.html
msfs2xp validate <apt.dat>
```

`INPUT` may be: `.bgl`, `.xml`, a package folder, a `Community`/`OneStore` folder,
or `auto:2020` / `auto:2024` (locate installed sims via `UserCfg.opt`).
Output layout per package: `<OUT>/<package>_XP12/Earth nav data/apt.dat` plus
`README.txt`; `--merge` writes one pack for all inputs. Progress via indicatif,
parallel packages via rayon, structured logs via tracing.

## 5. Parsing rules

### 5.1 Robustness contract
* No panics on any input: every read is bounds-checked; per-airport conversion is
  additionally wrapped in `catch_unwind`; failures become warnings attached to the
  airport, and the run continues.
* Unknown record ids are skipped by size and counted; `inspect` shows them.
* Files whose magic does not match are classified: `NotBgl`, `Encrypted`
  (high-entropy first 4 KiB, common for marketplace DRM), `Truncated`.
* Only sections of type `0x03` (airport) are parsed; `0x25` scenery objects are
  scanned only for windsocks/beacons when the layout matches.

### 5.2 Merging
Several BGLs in one package may contain the same ICAO (e.g. one with the airport,
another with jetways/paint). Airports with equal idents inside one package are
merged additively; `DeleteAirport` records are ignored (we convert only the
add-on itself). Across packages, later packages (Community after Official) win.

### 5.3 Variant detection
Detected once per airport and overridable with `--sim`:
* package `manifest.json` (`minimum_game_version` ≥ 2.0 or name starts `fs24-`) → 2024
* runway record: fixed-part length before first subrecord (FSX 0x38 bytes, MSFS
  0x38+0x2C) determined by probing for a valid subrecord id at each candidate
  offset; parking/path: element stride = (record size − header) / count matched to
  the known strides (FSX 0x24/0x14, MSFS 0x38/0x30, …).
* If nothing matches, use the largest stride that divides evenly, parse the common
  prefix only, and warn.

## 6. Conversion rules (MSFS → apt.dat 1200)

* **Airport row**: `1` land, `16` if all runways water, `17` if only helipads.
  Elevation m→ft. `1302` metadata: `icao_code`, `city`, `country`, `state`,
  `region_code`, `datum_lat/lon`, `flatten` (applyFlatten), `gui_label`,
  `transition_alt` omitted. Name from BGL name record.
* **Runway `100`**: width m; surface map (concrete/cement→2, asphalt/bituminous/
  tarmac/macadam/oil-treated→1, grass→3, dirt/clay/sand/shale/coral→4, gravel→5,
  snow/ice→14, water→water runway `101`, transparent→15); shoulder from
  edgePavement (1 asphalt/2 concrete); smoothness 0.25; centerline lights;
  edge lights NONE/LOW/MEDIUM/HIGH→0/1/2/3; distance signs 0. Ends: number +
  designator (37–44 → N/NE/…; L/R/C/W/A/B); positions from centre ± length/2 along
  true heading via Vincenty; displaced threshold = OffsetThreshold; blast pad =
  max(BlastPad, Overrun); markings: precision→3 (alternate*→7), touchdown or
  fixedDistance→2 (alternate→6), threshold or ident→1, none→0; closed end →
  marking 0; approach lights ALSF1→1, ALSF2→2, CALVERT→3,
  CALVERT2→4, SSALR→5, SSALF→6, SALS/SSALS/SALSF→7, MALSR→8, MALSF→9, MALS→10,
  ODALS→11, RAIL→12; TDZ from touchdown flag; REIL 1 if reil flag.
* **VASI `21`**: per end per side: PAPI2/PAPI4→2 (left)/3 (right), VASI*→1,
  TRICOLOR→5, APAP/BALL→7/8; placed 300 m past threshold, ±(width/2 + 15 m)
  lateral, heading = approach heading, angle = pitch.
* **Helipad `102`**: ident `H1…`, surface map, markings 1 unless type NONE.
* **Pavement `110`**: aprons with `drawSurface` → polygon rows (111/113). Taxi
  paths with `drawSurface` → for each segment a rectangle of `width`, plus a
  16-gon disc at every node shared by ≥2 drawn paths; grouped by (surface, taxi
  name) and unioned in the local tangent plane with `geo::BooleanOps`; if union
  fails the raw quads are emitted. `--no-union` skips the union.
* **Painted lines `120`**: TaxiwayPath centerLine → line 1 (+ light 101 if lit);
  left/right edge SOLID→1, DASHED→2, SOLID_DASHED→3, offset ±width/2 (+ light 102
  if lit); PaintedLine records: HOLD_SHORT_*→4, ILS_HOLD_SHORT→6, EDGE_LINE_
  SOLID→1, EDGE_LINE_DASHED→2, WIDE_YELLOW→8, NON_MOVEMENT→3, ENHANCED_CENTER→1,
  WIDE_WHITE→20, SERVICE/EDGE_SERVICE→22/20, WIDE_RED/SLIM_RED→5; hold-short
  taxi points → perpendicular 4/6 line of the path width (+ light 104/105);
  apron edge lights → 120 with light 102; blast/boundary fences are dropped
  (X-Plane `130` boundary has different semantics).
* **Taxi network `1200`**: nodes = taxi points + parking spots (+ runway path
  nodes); usage `both` for parkings, `junc` otherwise (`dest`/`init` unused).
  Edges from TAXI/PARKING/PATH (`taxiway`, width code from width: <7.5 A, <12 B,
  <18 C, <26 D, <32.5 E, else F) and RUNWAY (`runway`, name from number/
  designator). Active zones: BFS from each runway edge through taxi edges,
  stopping at hold-short points; visited taxi edges get `1204 departure,arrival
  RWY` (ILS hold-short → `1204 ils`). Names via TaxiName index. CLOSED, VEHICLE,
  ROAD paths → `1206` ground-truck edges. Degenerate edges (same node, zero
  length) dropped.
* **Ramp starts `1300/1301`**: GATE_*→`gate`, RAMP_GA*/DOCK_GA→`tie_down`,
  RAMP_CARGO/MIL→`misc`, FUEL/VEHICLE/NONE dropped. Aircraft types from radius
  (heavy ≥ 26 m, jets ≥ 12, turboprops ≥ 9, else props; helos for DOCK/H).
  Width code from radius as above; op type airline/general_aviation/cargo/military;
  airlines from airline codes; name `"<Name> <number><suffix>"` (GATE_A 12 →
  `A12`, PARKING 5 → `Parking 5`).
* **COM `1050–1056`**: ATIS/AWOS/ASOS→1050, UNICOM/CTAF/MULTICOM→1051,
  CLEARANCE(_PRE_TAXI)/REMOTE_CLEARANCE→1052, GROUND→1053, TOWER→1054,
  APPROACH→1055, DEPARTURE→1056; centre/FSS dropped; kHz integer.
* **Tower `14`**: height = tower alt − airport alt (ft, min 10), draw 1 if a
  tower object record exists. **Windsock `19`/beacon `18`**: XML only.
* **Signs `20`**: MSFS label → X-Plane text (table in §2); size 1–5 mapped 1:1
  (4→4 large distance-remaining, 5→3); heading = sign heading, justification
  ignored (X-Plane centres on position).
* Every dropped or approximated feature is counted and shown in the summary.

## 7. Error handling

`thiserror` error enum per layer; `anyhow` at the CLI edge. Warnings are
collected in `ConversionReport { airport, warnings: Vec<Warning>, stats }` and
printed as a table; exit code 0 if ≥1 airport written, 2 if none, 1 on I/O error.

## 8. Testing

* Unit tests: reader, ICAO/coordinate decoders, each record parser (against
  hand-built byte arrays from a test `BglBuilder`), XML parser, every mapping
  table, geodesy (round trip, known distances), polygon union, sign translator,
  network active-zone BFS, apt.dat writer ↔ validator round trip.
* Golden tests: synthetic BGL and XML fixtures under `tests/fixtures/` produce
  byte-exact apt.dat under `tests/golden/`.
* Real-data integration test (ignored unless `MSFS2XP_FIXTURES` points at the
  MSFS 2020 Official/OneStore folder): converts all `asobo-airport-*` packages
  and 200 random generic-airport BGLs; asserts no panics, runway counts, validator
  passes, and no coordinate farther than 15 km from the airport datum.
* Quality gates: `cargo test`, `cargo clippy -D warnings`, `cargo fmt --check`.

## 9. Dependencies

clap 4 (derive), roxmltree, geo 0.30+, thiserror, anyhow, rayon, indicatif,
tracing + tracing-subscriber, serde + serde_json, walkdir, insta (dev, golden
tests), proptest (dev, geodesy).
