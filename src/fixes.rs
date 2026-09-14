//! Texture fixes chosen by hand: flips and hidden textures, remembered per
//! pack in `msfs2xp-fixes.json`.
//!
//! A fix is keyed by the texture's name up to its first dot, lower-cased, so
//! the same entry matches the MSFS source (`ROOF_ALBD.PNG.KTX2`) and the
//! converted file (`ROOF_ALBD.dds`). The converter applies the file while it
//! writes a pack, and [`apply_to_pack`] applies one fix to a pack already on
//! disk. The file sits at the pack root, which a re-conversion leaves alone.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::texture;

/// Name of the fixes file at the root of a pack.
pub const FILE_NAME: &str = "msfs2xp-fixes.json";

/// What to do to one texture. Flipping both ways is a 180 degree turn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureFix {
    #[serde(default)]
    pub flip_v: bool,
    #[serde(default)]
    pub flip_h: bool,
    /// Draw nothing that uses this texture.
    #[serde(default)]
    pub hide: bool,
}

impl TextureFix {
    pub fn is_none(&self) -> bool {
        !(self.flip_v || self.flip_h || self.hide)
    }

    pub fn flips(&self) -> bool {
        self.flip_v || self.flip_h
    }
}

/// Every fix for one pack.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Fixes {
    #[serde(default)]
    pub textures: BTreeMap<String, TextureFix>,
}

/// The key a texture's fix is stored under.
pub fn key(texture_file: &str) -> String {
    let name = texture_file.rsplit(['/', '\\']).next().unwrap_or(texture_file);
    name.split('.').next().unwrap_or(name).to_ascii_lowercase()
}

impl Fixes {
    /// Read a fixes file; a missing file means no fixes.
    pub fn load(path: &Path) -> anyhow::Result<Fixes> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Fixes::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn get(&self, texture_file: &str) -> TextureFix {
        self.textures.get(&key(texture_file)).copied().unwrap_or_default()
    }

    pub fn set(&mut self, texture_file: &str, fix: TextureFix) {
        if fix.is_none() {
            self.textures.remove(&key(texture_file));
        } else {
            self.textures.insert(key(texture_file), fix);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }
}

/// An object that draws nothing, standing in for a hidden one. It keeps its
/// TEXTURE line so the texture it belonged to can still be found.
fn blank_object(texture_line: &str) -> String {
    format!("I\n800\nOBJ\n\n{texture_line}\nPOINT_COUNTS 0 0 0 0\n")
}

/// The TEXTURE line of an object file, read from its header.
fn texture_line(obj: &str) -> Option<&str> {
    obj.lines()
        .take_while(|l| !l.starts_with("POINT_COUNTS"))
        .find(|l| l.starts_with("TEXTURE ") || l.starts_with("TEXTURE\t"))
}

/// Apply one texture's fix to a converted pack in place and remember it in
/// the pack's fixes file. The texture is rebuilt from a backup of the
/// converted original (kept in `objects/textures/_originals`), so fixes can
/// be changed or undone any number of times. Hiding blanks every object
/// whose base texture it is (backed up in `objects/_originals`); unhiding
/// restores them. Returns how many objects were hidden or restored.
pub fn apply_to_pack(pack: &Path, texture_file: &str, fix: TextureFix) -> anyhow::Result<usize> {
    let objects = pack.join("objects");
    let textures = objects.join("textures");
    let path = textures.join(texture_file);
    let backup_dir = textures.join("_originals");
    let backup = backup_dir.join(texture_file);
    if !backup.exists() {
        std::fs::create_dir_all(&backup_dir)?;
        std::fs::copy(&path, &backup)?;
    }
    let original = std::fs::read(&backup)?;
    let bytes = if fix.flips() {
        texture::transform_texture_file(&original, fix.flip_v, fix.flip_h)?
    } else {
        original
    };
    std::fs::write(&path, bytes)?;

    let wanted = format!("TEXTURE textures/{texture_file}");
    let obj_backups = objects.join("_originals");
    let mut changed = 0;
    for e in std::fs::read_dir(&objects)? {
        let e = e?;
        let p = e.path();
        if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("obj")) {
            continue;
        }
        let text = std::fs::read_to_string(&p)?;
        if texture_line(&text).map(str::trim_end) != Some(wanted.as_str()) {
            continue;
        }
        let saved = obj_backups.join(e.file_name());
        if fix.hide {
            if !saved.exists() {
                std::fs::create_dir_all(&obj_backups)?;
                std::fs::write(&saved, &text)?;
            }
            std::fs::write(&p, blank_object(&wanted))?;
            changed += 1;
        } else if saved.exists() {
            std::fs::copy(&saved, &p)?;
            std::fs::remove_file(&saved)?;
            changed += 1;
        }
    }

    let fixes_path = pack.join(FILE_NAME);
    let mut fixes = Fixes::load(&fixes_path)?;
    fixes.set(texture_file, fix);
    fixes.save(&fixes_path)?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_ignore_extension_and_case() {
        assert_eq!(key("ROOF_ALBD.PNG.KTX2"), "roof_albd");
        assert_eq!(key("textures/Roof_Albd.dds"), "roof_albd");
        let mut f = Fixes::default();
        f.set("ROOF_ALBD.PNG.KTX2", TextureFix { flip_v: true, ..Default::default() });
        assert!(f.get("roof_albd.dds").flip_v, "the source name's fix matches the converted file");
        f.set("roof_albd.dds", TextureFix::default());
        assert!(f.is_empty(), "clearing a fix removes it");
    }

    #[test]
    fn fixes_round_trip_through_their_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        assert!(Fixes::load(&path).unwrap().is_empty(), "no file, no fixes");
        let mut f = Fixes::default();
        f.set("A.dds", TextureFix { flip_h: true, hide: true, ..Default::default() });
        f.save(&path).unwrap();
        assert_eq!(Fixes::load(&path).unwrap(), f);
    }

    /// A 2x1 RGBA PNG: red on the left, blue on the right.
    fn two_pixels() -> Vec<u8> {
        let img = image::RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn fixes_apply_to_a_pack_and_undo() {
        let dir = tempfile::tempdir().unwrap();
        let pack = dir.path();
        std::fs::create_dir_all(pack.join("objects/textures")).unwrap();
        std::fs::write(pack.join("objects/textures/A.png"), two_pixels()).unwrap();
        let obj = "I\n800\nOBJ\n\nTEXTURE textures/A.png\nPOINT_COUNTS 3 0 0 3\n\nVT 0 0 0 0 1 0 0 0\n";
        std::fs::write(pack.join("objects/x_0.obj"), obj).unwrap();
        std::fs::write(pack.join("objects/y_0.obj"), obj.replace("A.png", "B.png")).unwrap();

        apply_to_pack(pack, "A.png", TextureFix { flip_h: true, ..Default::default() }).unwrap();
        let img = image::load_from_memory(&std::fs::read(pack.join("objects/textures/A.png")).unwrap()).unwrap().to_rgba8();
        assert_eq!(img.get_pixel(0, 0).0, [0, 0, 255, 255], "blue now on the left");
        assert!(pack.join("objects/textures/_originals/A.png").exists());

        let hidden = apply_to_pack(pack, "A.png", TextureFix { hide: true, ..Default::default() }).unwrap();
        assert_eq!(hidden, 1, "only the object using A.png");
        let blank = std::fs::read_to_string(pack.join("objects/x_0.obj")).unwrap();
        assert!(blank.contains("POINT_COUNTS 0 0 0 0") && blank.contains("TEXTURE textures/A.png"));
        assert_eq!(std::fs::read_to_string(pack.join("objects/y_0.obj")).unwrap(), obj.replace("A.png", "B.png"));
        let img = image::load_from_memory(&std::fs::read(pack.join("objects/textures/A.png")).unwrap()).unwrap().to_rgba8();
        assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 255], "no flips: the original again");

        apply_to_pack(pack, "A.png", TextureFix::default()).unwrap();
        assert_eq!(std::fs::read_to_string(pack.join("objects/x_0.obj")).unwrap(), obj, "unhide restores the object");
        assert!(Fixes::load(&pack.join(FILE_NAME)).unwrap().is_empty());
    }
}
