//! The `convert`, `list` and `validate` commands.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Context;
use rayon::prelude::*;

use crate::bgl::records::Variant;
use crate::cli::{ConvertArgs, ListArgs, ValidateArgs};
use crate::convert::{self, Options, Report};
use crate::package::{self, Loaded, Source};
use crate::xplane::{self, apt::AptAirport, Apt};

fn variant_hint(sim: &str) -> Option<Variant> {
    match sim {
        "fsx" => Some(Variant::Fsx),
        "p3d" => Some(Variant::P3dV5),
        _ => None,
    }
}

/// Folder names X-Plane and Windows both accept.
pub fn pack_folder_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '-'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').to_string();
    format!(
        "{} (msfs2xp)",
        if cleaned.is_empty() {
            "Converted airport"
        } else {
            &cleaned
        }
    )
}

fn wanted(icao: &str, filter: &[String]) -> bool {
    filter.is_empty() || filter.iter().any(|f| f.eq_ignore_ascii_case(icao))
}

/// Convert every airport of a loaded source, isolating panics per airport.
fn convert_loaded(loaded: &Loaded, filter: &[String], opts: &Options) -> Vec<(AptAirport, Report)> {
    loaded
        .airports
        .iter()
        .filter(|a| wanted(&a.icao, filter))
        .filter(|a| {
            // X-Plane rejects an airport with no runway, water runway or helipad.
            let usable = !a.runways.is_empty() || !a.helipads.is_empty();
            if !usable {
                eprintln!("note: {} ({}) has no runway or helipad; skipped", a.icao, a.name);
            }
            usable
        })
        .filter_map(|ap| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| convert::convert(ap, opts)));
            match result {
                Ok(pair) => Some(pair),
                Err(_) => {
                    eprintln!("error: converting {} crashed; skipped", ap.icao);
                    None
                }
            }
        })
        .collect()
}

fn readme(source_titles: &[String], reports: &[Report], problems: &[String]) -> String {
    let mut s = String::new();
    s.push_str("Converted by msfs2xp from Microsoft Flight Simulator scenery.\r\n\r\n");
    s.push_str("Install: copy this whole folder into \"X-Plane 12/Custom Scenery\".\r\n");
    s.push_str("X-Plane adds it to scenery_packs.ini on the next start.\r\n\r\n");
    s.push_str("Source packages:\r\n");
    for t in source_titles {
        s.push_str(&format!("  - {t}\r\n"));
    }
    s.push_str("\r\nAirports:\r\n");
    for r in reports {
        s.push_str(&format!("\r\n{} {} ({})\r\n", r.icao, r.name, r.sim));
        for (k, v) in &r.converted {
            s.push_str(&format!("  converted {v:>6}  {k}\r\n"));
        }
        for (k, v) in &r.dropped {
            s.push_str(&format!("  skipped   {v:>6}  {k}\r\n"));
        }
    }
    if !problems.is_empty() {
        s.push_str("\r\nFiles that could not be read:\r\n");
        for p in problems {
            s.push_str(&format!("  - {p}\r\n"));
        }
    }
    s.push_str("\r\nThis pack is for personal use with scenery you own. Do not redistribute it.\r\n");
    s
}

/// Write one scenery pack and return its folder and validation problems.
fn write_pack(
    out_root: &Path,
    folder: &str,
    airports: Vec<AptAirport>,
    reports: &[Report],
    titles: &[String],
    problems: &[String],
) -> anyhow::Result<(PathBuf, Vec<String>)> {
    let dir = out_root.join(folder);
    let nav = dir.join("Earth nav data");
    std::fs::create_dir_all(&nav).with_context(|| format!("creating {}", nav.display()))?;
    let text = xplane::write(&Apt { airports });
    let issues = match xplane::validate(&text) {
        Ok(_) => Vec::new(),
        Err(e) => e,
    };
    std::fs::write(nav.join("apt.dat"), &text).context("writing apt.dat")?;
    std::fs::write(dir.join("README.txt"), readme(titles, reports, problems)).context("writing README")?;
    std::fs::write(dir.join("msfs2xp-report.json"), serde_json::to_string_pretty(reports)?)
        .context("writing report")?;
    Ok((dir, issues))
}

fn summary_row(r: &Report) -> String {
    let c = |k: &str| r.converted.get(k).copied().unwrap_or(0);
    format!(
        "{:<8} {:<34} {:>4} {:>6} {:>6} {:>6} {:>5} {:>6} {:>5}",
        r.icao,
        r.name.chars().take(34).collect::<String>(),
        c("runways") + c("water runways") + c("helipads"),
        c("pavement polygons"),
        c("painted lines") + c("taxiway centrelines") + c("taxiway edge lines") + c("hold-short bars"),
        c("taxi network edges"),
        c("ramp starts"),
        c("jetways"),
        r.warnings.len()
    )
}

pub fn run_convert(args: &ConvertArgs) -> anyhow::Result<i32> {
    let started = Instant::now();
    if let Some(jobs) = args.jobs {
        let _ = rayon::ThreadPoolBuilder::new().num_threads(jobs.max(1)).build_global();
    }
    let mut sources: Vec<Source> = Vec::new();
    for input in &args.inputs {
        sources.extend(package::discover(input)?);
    }
    let opts = Options {
        union: !args.no_union,
        network: !args.no_network,
        lines: !args.no_lines,
    };
    if args.preview {
        eprintln!("note: --preview is not implemented yet; open the pack in WorldEditor to inspect it");
    }
    let hint = variant_hint(&args.sim);

    let results: Vec<(Loaded, Vec<(AptAirport, Report)>)> = sources
        .par_iter()
        .map(|src| {
            let loaded = package::load(src, hint);
            let converted = convert_loaded(&loaded, &args.icao, &opts);
            (loaded, converted)
        })
        .collect();

    std::fs::create_dir_all(&args.out).with_context(|| format!("creating {}", args.out.display()))?;
    println!(
        "{:<8} {:<34} {:>4} {:>6} {:>6} {:>6} {:>5} {:>6} {:>5}",
        "ICAO", "Name", "Rwy", "Pave", "Lines", "Edges", "Ramps", "Jetwy", "Warn"
    );
    let mut written = 0usize;
    let mut invalid = 0usize;
    let mut packs = Vec::new();

    let mut emit = |folder: String,
                    pairs: Vec<(AptAirport, Report)>,
                    titles: Vec<String>,
                    problems: Vec<String>|
     -> anyhow::Result<()> {
        if pairs.is_empty() {
            return Ok(());
        }
        let (apts, reports): (Vec<_>, Vec<_>) = pairs.into_iter().unzip();
        for r in &reports {
            println!("{}", summary_row(r));
        }
        let (dir, issues) = write_pack(&args.out, &folder, apts, &reports, &titles, &problems)?;
        if !issues.is_empty() {
            invalid += 1;
            eprintln!(
                "warning: {} failed validation ({} issues):",
                dir.display(),
                issues.len()
            );
            for i in issues.iter().take(15) {
                eprintln!("  {i}");
            }
        }
        written += reports.len();
        packs.push(dir);
        Ok(())
    };

    if args.merge {
        let mut all = Vec::new();
        let mut titles = Vec::new();
        let mut problems = Vec::new();
        for (loaded, pairs) in results {
            titles.push(loaded.source.display_name());
            problems.extend(loaded.problems);
            all.extend(pairs);
        }
        emit(pack_folder_name("msfs2xp merged airports"), all, titles, problems)?;
        if !args.no_objects {
            eprintln!("note: --merge writes airport layouts only; run without --merge to convert buildings");
        }
    } else {
        for (loaded, pairs) in results {
            for p in &loaded.problems {
                eprintln!("note: {}: {p}", loaded.source.name);
            }
            for n in &loaded.notes {
                eprintln!("note: {n}");
            }
            if pairs.is_empty() {
                eprintln!("note: {} contains no convertible airport", loaded.source.name);
            }
            let title = loaded.source.display_name();
            let has_airports = !pairs.is_empty();
            let folder = pack_folder_name(&title);
            emit(folder.clone(), pairs, vec![title], loaded.problems.clone())?;
            if has_airports && !args.no_objects {
                let dir = args.out.join(&folder);
                let began = Instant::now();
                let opts = crate::objects::ObjectOptions {
                    lod: args.lod,
                    max_triangles: args.max_tris,
                };
                match crate::objects::build(&loaded, &dir, &opts) {
                    Ok(r) => {
                        println!(
                            "  objects: {} of {} placements from {} models: {} object files, {} textures, {:.1}M triangles, {} DSF tile(s), {:.0}s",
                            r.placed,
                            r.placements,
                            r.models_converted,
                            r.object_files,
                            r.textures_written,
                            r.triangles as f64 / 1e6,
                            r.dsf_tiles.len(),
                            began.elapsed().as_secs_f64()
                        );
                        if r.stock_placements > 0 || r.not_in_package > 0 || !r.failed_models.is_empty() {
                            println!(
                                "  objects: {} placements use MSFS 2020 stock models; {} use stock models not on this PC (MSFS 2024 streams them); {} models failed",
                                r.stock_placements,
                                r.not_in_package,
                                r.failed_models.len()
                            );
                        }
                        std::fs::write(dir.join("msfs2xp-objects.json"), serde_json::to_string_pretty(&r)?)?;
                    }
                    Err(e) => eprintln!("warning: objects for {folder} failed: {e:#}"),
                }
            }
        }
    }

    for p in &packs {
        println!("wrote {}", p.display());
    }
    println!("{written} airport(s) in {:.1}s", started.elapsed().as_secs_f64());
    Ok(if written == 0 {
        2
    } else if invalid > 0 {
        3
    } else {
        0
    })
}

pub fn run_list(args: &ListArgs) -> anyhow::Result<i32> {
    let mut total = 0;
    for input in &args.inputs {
        for src in package::discover(input)? {
            let loaded = package::load(&src, None);
            println!(
                "{}  [{}]  {} BGL files, {} model libraries, {} placed objects",
                src.display_name(),
                src.sim.label(),
                src.files.len(),
                loaded.model_libraries.len(),
                loaded.placements.len()
            );
            for a in loaded.airports.iter().filter(|a| wanted(&a.icao, &args.icao)) {
                total += 1;
                println!(
                    "  {:<8} {:<34} runways {:>2}  stands {:>4}  taxi paths {:>5}  aprons {:>6}  lines {:>5}",
                    a.icao,
                    a.name.chars().take(34).collect::<String>(),
                    a.runways.len(),
                    a.parkings.len(),
                    a.taxi_paths.len(),
                    a.aprons.len(),
                    a.painted_lines.len()
                );
            }
            for p in &loaded.problems {
                println!("  note: {p}");
            }
        }
    }
    Ok(if total == 0 { 2 } else { 0 })
}

pub fn run_validate(args: &ValidateArgs) -> anyhow::Result<i32> {
    let text = std::fs::read_to_string(&args.apt).with_context(|| format!("reading {}", args.apt.display()))?;
    match xplane::validate(&text) {
        Ok(r) => {
            println!(
                "valid: {} airport(s), {} rows, {} runways, {} pavements, {} lines, {} network nodes, {} edges, {} ramps, {} jetways",
                r.airports, r.rows, r.runways, r.pavements, r.lines, r.network_nodes, r.network_edges, r.ramp_starts, r.jetways
            );
            Ok(0)
        }
        Err(errors) => {
            for e in &errors {
                println!("{e}");
            }
            println!("{} problem(s)", errors.len());
            Ok(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_names_are_filesystem_safe() {
        assert_eq!(
            pack_folder_name("KORD Chicago O'Hare International"),
            "KORD Chicago O'Hare International (msfs2xp)"
        );
        assert_eq!(pack_folder_name("A/B: C?"), "A-B- C- (msfs2xp)");
        assert_eq!(pack_folder_name("  "), "Converted airport (msfs2xp)");
    }
}
