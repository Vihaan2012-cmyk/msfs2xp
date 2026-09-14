//! Everything the window does that is not drawing it: remembered settings,
//! the command lines it runs, reading `msfs2xp list` output, and texture
//! thumbnails. Kept free of UI code so it can be tested anywhere.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use msfs2xp::fixes::TextureFix;
use msfs2xp::texture;

/// What the Convert tab remembers between runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub packages: Vec<String>,
    pub output: String,
    pub objects: bool,
    pub normal_maps: bool,
    pub merge: bool,
    pub lines: bool,
    pub network: bool,
    pub union: bool,
    pub max_texture: u32,
    pub normal_max: u32,
    pub sim: String,
}

impl Default for Settings {
    fn default() -> Self {
        let downloads = std::env::var("USERPROFILE")
            .map(|home| Path::new(&home).join("Downloads").join("msfs2xp"))
            .unwrap_or_else(|_| PathBuf::from("msfs2xp-output"));
        Settings {
            packages: Vec::new(),
            output: downloads.display().to_string(),
            objects: true,
            normal_maps: false,
            merge: false,
            lines: true,
            network: true,
            union: true,
            max_texture: 2048,
            normal_max: 512,
            sim: "auto".into(),
        }
    }
}

/// Where the settings live: `%APPDATA%\msfs2xp\gui.json`.
pub fn settings_path() -> Option<PathBuf> {
    std::env::var("APPDATA").ok().map(|d| Path::new(&d).join("msfs2xp").join("gui.json"))
}

pub fn load_settings() -> Settings {
    settings_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_settings(s: &Settings) {
    if let Some(p) = settings_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(s) {
            let _ = std::fs::write(p, text);
        }
    }
}

/// The command line tool, expected next to this program.
pub fn cli_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("msfs2xp.exe")))
        .unwrap_or_else(|| PathBuf::from("msfs2xp.exe"))
}

/// Arguments for `msfs2xp convert` from the Convert tab. An empty `icaos`
/// converts every airport found.
pub fn convert_args(s: &Settings, icaos: &[String]) -> Vec<String> {
    let mut a = vec!["convert".to_string()];
    a.extend(s.packages.iter().cloned());
    a.extend(["-o".to_string(), s.output.clone()]);
    if !icaos.is_empty() {
        a.extend(["--icao".to_string(), icaos.join(",")]);
    }
    if !s.objects {
        a.push("--no-objects".into());
    }
    if s.normal_maps {
        a.extend(["--normal-maps".to_string(), "--normal-max".to_string(), s.normal_max.to_string()]);
    }
    if s.merge {
        a.push("--merge".into());
    }
    if !s.lines {
        a.push("--no-lines".into());
    }
    if !s.network {
        a.push("--no-network".into());
    }
    if !s.union {
        a.push("--no-union".into());
    }
    a.extend(["--max-texture".to_string(), s.max_texture.to_string()]);
    if s.sim != "auto" {
        a.extend(["--sim".to_string(), s.sim.clone()]);
    }
    a
}

/// `(ICAO, name)` of every airport in `msfs2xp list` output.
pub fn parse_list(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|l| l.starts_with("  ") && l.contains(" runways "))
        .filter_map(|l| {
            let l = l.trim_start();
            let (icao, rest) = l.split_once(char::is_whitespace)?;
            let name = rest.split(" runways ").next()?.trim();
            Some((icao.to_string(), name.to_string()))
        })
        .collect()
}

/// The converted textures of a pack, sorted by name (backups left out).
pub fn list_textures(pack: &Path) -> std::io::Result<Vec<String>> {
    let mut out: Vec<String> = std::fs::read_dir(pack.join("objects").join("textures"))?
        .filter_map(Result::ok)
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| {
            let n = n.to_ascii_lowercase();
            n.ends_with(".dds") || n.ends_with(".png")
        })
        .collect();
    out.sort_by_key(|n| n.to_ascii_lowercase());
    Ok(out)
}

/// A square PNG thumbnail of a converted texture, shown the right way up
/// (the converter stores DDS textures bottom row first, as X-Plane reads
/// them) over a dark background so transparent parts stay visible.
pub fn thumbnail_png(path: &Path, side: u32) -> anyhow::Result<Vec<u8>> {
    let data = std::fs::read(path)?;
    let (w, h, rgba) = texture::decode_within(&data, side.max(4))?;
    let mut img = image::RgbaImage::from_raw(w, h, rgba).ok_or_else(|| anyhow::anyhow!("bad texture size"))?;
    if texture::detect(&data) == texture::SourceFormat::Dds {
        img = image::imageops::flip_vertical(&img);
    }
    let k = side as f64 / w.max(h) as f64;
    let (tw, th) = (((w as f64 * k).round() as u32).max(1), ((h as f64 * k).round() as u32).max(1));
    let img = image::imageops::resize(&img, tw, th, image::imageops::FilterType::Triangle);
    let mut canvas = image::RgbaImage::from_pixel(side, side, image::Rgba([38, 38, 42, 255]));
    let (ox, oy) = ((side - tw) / 2, (side - th) / 2);
    for (x, y, p) in img.enumerate_pixels() {
        let a = p[3] as u32;
        let bg = canvas.get_pixel(ox + x, oy + y).0;
        let mix = |c: usize| ((p[c] as u32 * a + bg[c] as u32 * (255 - a)) / 255) as u8;
        canvas.put_pixel(ox + x, oy + y, image::Rgba([mix(0), mix(1), mix(2), 255]));
    }
    let mut out = Vec::new();
    canvas.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?;
    Ok(out)
}

/// One line about a converted texture: format, size, levels, file size.
pub fn texture_info(path: &Path) -> String {
    let Ok(data) = std::fs::read(path) else {
        return "unreadable".into();
    };
    let mb = data.len() as f64 / 1e6;
    let u32_at = |o: usize| data.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0);
    match texture::detect(&data) {
        texture::SourceFormat::Dds => {
            let fourcc = data.get(84..88).map(|b| String::from_utf8_lossy(b).to_string()).unwrap_or_default();
            format!("DDS {} \u{b7} {}\u{d7}{} \u{b7} {} mip levels \u{b7} {:.1} MB", fourcc.trim_matches('\0'), u32_at(16), u32_at(12), u32_at(28).max(1), mb)
        }
        texture::SourceFormat::Png => {
            let be = |o: usize| data.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0);
            format!("PNG \u{b7} {}\u{d7}{} \u{b7} {:.1} MB", be(16), be(20), mb)
        }
        _ => format!("{:.1} MB", mb),
    }
}

/// The fix on a texture, in words.
pub fn describe(fix: &TextureFix) -> String {
    let mut parts = Vec::new();
    match (fix.flip_v, fix.flip_h) {
        (true, true) => parts.push("turned 180\u{b0}"),
        (true, false) => parts.push("flipped top to bottom"),
        (false, true) => parts.push("flipped left to right"),
        _ => {}
    }
    if fix.hide {
        parts.push("hidden");
    }
    if parts.is_empty() {
        "No fix".into()
    } else {
        format!("Fix: {}", parts.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_arguments_follow_the_options() {
        let mut s = Settings {
            packages: vec!["D:\\pkg".into(), "auto:2024".into()],
            output: "C:\\out".into(),
            ..Settings::default()
        };
        assert_eq!(
            convert_args(&s, &[]),
            ["convert", "D:\\pkg", "auto:2024", "-o", "C:\\out", "--max-texture", "2048"]
        );
        s.objects = false;
        s.normal_maps = true;
        s.merge = true;
        s.lines = false;
        s.network = false;
        s.union = false;
        s.max_texture = 1024;
        s.normal_max = 256;
        s.sim = "2020".into();
        let a = convert_args(&s, &["OMDB".into(), "KORD".into()]);
        for want in [
            "--icao", "OMDB,KORD", "--no-objects", "--normal-maps", "--normal-max", "256", "--merge", "--no-lines",
            "--no-network", "--no-union", "1024", "--sim", "2020",
        ] {
            assert!(a.iter().any(|x| x == want), "{want} missing from {a:?}");
        }
    }

    #[test]
    fn list_output_gives_airports() {
        let text = "OMDB Dubai International Airport  [MSFS 2020]  9 BGL files, 2 model libraries, 15770 placed objects\n  OMDB     Dubai Intl                         runways  2  stands  219  taxi paths  5341  aprons  24789  lines  7569\n  note: something\n";
        assert_eq!(parse_list(text), vec![("OMDB".to_string(), "Dubai Intl".to_string())]);
    }

    #[test]
    fn fixes_are_described_in_words() {
        assert_eq!(describe(&TextureFix::default()), "No fix");
        let both = TextureFix { flip_v: true, flip_h: true, hide: true };
        assert_eq!(describe(&both), "Fix: turned 180\u{b0}, hidden");
    }

    #[test]
    fn thumbnails_are_square_pngs_and_listing_skips_backups() {
        let dir = tempfile::tempdir().unwrap();
        let tex = dir.path().join("objects").join("textures");
        std::fs::create_dir_all(tex.join("_originals")).unwrap();
        let img = image::RgbaImage::from_pixel(8, 4, image::Rgba([200, 10, 10, 255]));
        img.save(tex.join("B.png")).unwrap();
        img.save(tex.join("_originals").join("B.png")).unwrap();
        std::fs::write(tex.join("notes.txt"), "x").unwrap();
        assert_eq!(list_textures(dir.path()).unwrap(), vec!["B.png".to_string()]);
        let png = thumbnail_png(&tex.join("B.png"), 64).unwrap();
        let thumb = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(thumb.dimensions(), (64, 64));
        assert_eq!(thumb.get_pixel(32, 32).0, [200, 10, 10, 255], "the texture fills the middle");
        assert!(texture_info(&tex.join("B.png")).starts_with("PNG \u{b7} 8\u{d7}4"));
    }
}
