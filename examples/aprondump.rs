//! Development helper: the aprons near a point, in drawing order, as JSON
//! (priority, surface, material, flags, drawn or decal, vertices), to draw
//! next to the converted pavement.
//!
//! usage: aprondump <package folder> <lat> <lon> <half size m> <out.json>
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let (input, lat, lon, half, out) = (
        &args[1],
        args[2].parse::<f64>()?,
        args[3].parse::<f64>()?,
        args[4].parse::<f64>()?,
        &args[5],
    );
    let (dlat, dlon) = (half / 110_540.0, half / (111_320.0 * lat.to_radians().cos()));
    let mut rows = Vec::new();
    for source in msfs2xp::package::discover(input)? {
        let loaded = msfs2xp::package::load(&source, None);
        for ap in &loaded.airports {
            for (i, a) in ap.aprons.iter().enumerate() {
                let near = a.vertices.iter().any(|v| (v.lat - lat).abs() < dlat && (v.lon - lon).abs() < dlon);
                if !near {
                    continue;
                }
                rows.push(serde_json::json!({
                    "order": i,
                    "priority": a.priority,
                    "surface": format!("{:?}", a.surface),
                    "material": a.material_name,
                    "guid": a.material_guid.map(|g| g.to_string()),
                    "flags": a.flags,
                    "tint": format!("{:?}", a.tint),
                    "uv_scale": a.uv_scale,
                    "uv_rotation": a.uv_rotation,
                    "texture": a.decal_texture,
                    "draw": a.draw,
                    "vertices": a.vertices.iter().map(|v| [v.lat, v.lon]).collect::<Vec<_>>(),
                }));
            }
        }
    }
    std::fs::write(out, serde_json::to_string(&rows)?)?;
    println!("{} aprons near {lat},{lon}", rows.len());
    Ok(())
}
