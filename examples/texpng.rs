//! Development helper: decode textures to PNG in their stored orientation.
fn main() -> anyhow::Result<()> {
    let out = std::path::PathBuf::from(std::env::args().nth(1).expect("output folder"));
    std::fs::create_dir_all(&out)?;
    for p in std::env::args().skip(2) {
        let data = std::fs::read(&p)?;
        let img = msfs2xp::texture::load(&data).map_err(|e| anyhow::anyhow!("{p}: {e}"))?;
        let rgba = msfs2xp::texture::decode_rgba8(&img, 0).map_err(|e| anyhow::anyhow!("{p}: {e}"))?;
        let name = std::path::Path::new(&p).file_name().unwrap().to_string_lossy().replace(".ktx2", "").replace(".KTX2", "");
        let f = out.join(format!("{name}.png"));
        image::RgbaImage::from_raw(img.width, img.height, rgba).ok_or_else(|| anyhow::anyhow!("size"))?.save(&f)?;
        println!("{} {}x{} {:?}", f.display(), img.width, img.height, img.format);
    }
    Ok(())
}
