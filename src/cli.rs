//! Command line interface definition.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Convert Microsoft Flight Simulator airports into X-Plane 12 scenery.
#[derive(Parser, Debug)]
#[command(name = "msfs2xp", version, about, long_about = None)]
pub struct Cli {
    /// Increase log verbosity (repeatable).
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Convert one or more BGL files, XML files or packages into X-Plane scenery packs.
    Convert(ConvertArgs),
    /// List the airports found in the given inputs without writing anything.
    List(ListArgs),
    /// Dump the record structure of a BGL file (for reverse engineering).
    Inspect(InspectArgs),
    /// Render an HTML/SVG preview of an apt.dat file.
    Preview(PreviewArgs),
    /// Check that an apt.dat file is structurally valid.
    Validate(ValidateArgs),
}

#[derive(clap::Args, Debug)]
pub struct ConvertArgs {
    /// Inputs: .bgl, .xml, a package folder, a Community/OneStore folder, or auto:2020 / auto:2024.
    #[arg(required = true)]
    pub inputs: Vec<String>,

    /// Output directory that will hold the generated scenery pack(s).
    #[arg(short, long)]
    pub out: PathBuf,

    /// Only convert these ICAO identifiers (comma separated).
    #[arg(long, value_delimiter = ',')]
    pub icao: Vec<String>,

    /// Write every airport into one merged scenery pack.
    #[arg(long)]
    pub merge: bool,

    /// Force a record layout instead of detecting it.
    #[arg(long, default_value = "auto", value_parser = ["auto", "2020", "2024", "fsx", "p3d"])]
    pub sim: String,

    /// Do not union taxiway/apron polygons (faster, more rows).
    #[arg(long)]
    pub no_union: bool,

    /// Do not generate the ATC taxi network.
    #[arg(long)]
    pub no_network: bool,

    /// Do not generate painted lines and lights.
    #[arg(long)]
    pub no_lines: bool,

    /// Also write a preview.html next to each apt.dat.
    #[arg(long)]
    pub preview: bool,

    /// Also write airports.json with the intermediate model.
    #[arg(long)]
    pub json: bool,

    /// Number of packages to convert in parallel.
    #[arg(short, long)]
    pub jobs: Option<usize>,

    /// Skip converting the 3D buildings and other placed objects.
    #[arg(long)]
    pub no_objects: bool,

    /// Model level of detail to convert; 0 is the most detailed.
    #[arg(long, default_value_t = 0)]
    pub lod: usize,

    /// Step down to coarser model LODs until each model has at most this many triangles.
    #[arg(long, default_value_t = 500_000)]
    pub max_tris: usize,

    /// Largest texture side, for terminals; hangars get half, vehicles a quarter, people an eighth.
    #[arg(long, default_value_t = 2048)]
    pub max_texture: u32,

    /// Also convert normal maps (surface detail) for buildings 15 m and larger.
    /// Costs video memory: X-Plane keeps normal maps uncompressed.
    #[arg(long)]
    pub normal_maps: bool,

    /// Largest normal map side with --normal-maps (they stay uncompressed in X-Plane).
    #[arg(long, default_value_t = 512)]
    pub normal_max: u32,

    /// Texture fixes file (flips, hidden textures). By default each pack's own
    /// msfs2xp-fixes.json is used.
    #[arg(long)]
    pub fixes: Option<PathBuf>,
}

#[derive(clap::Args, Debug)]
pub struct ListArgs {
    /// Inputs, as for `convert`.
    #[arg(required = true)]
    pub inputs: Vec<String>,

    /// Only list these ICAO identifiers (comma separated).
    #[arg(long, value_delimiter = ',')]
    pub icao: Vec<String>,
}

#[derive(clap::Args, Debug)]
pub struct InspectArgs {
    /// BGL file to inspect.
    pub file: PathBuf,

    /// Restrict output to one airport.
    #[arg(long)]
    pub icao: Option<String>,

    /// Hex dump unknown records.
    #[arg(long)]
    pub hex: bool,
}

#[derive(clap::Args, Debug)]
pub struct PreviewArgs {
    /// apt.dat file to render.
    pub apt: PathBuf,

    /// Output HTML file.
    #[arg(short, long)]
    pub out: PathBuf,
}

#[derive(clap::Args, Debug)]
pub struct ValidateArgs {
    /// apt.dat file to check.
    pub apt: PathBuf,
}
