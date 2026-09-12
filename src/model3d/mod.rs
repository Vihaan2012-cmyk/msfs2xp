//! probe
#![allow(clippy::all)]

#[cfg(test)]
mod tests {
    #[test]
    fn probe() {
        let p = std::path::Path::new("C:/Users/bansa/AppData/Local/Packages/Microsoft.Limitless_8wekyb3d8bbwe/LocalCache/Packages/Community/fnx-aircraft-320/SimObjects/Airplanes/FNX_32X/attachments/fnx/Part_Exterior_GPU/model/FNX_32X_Exterior_GSE_GPU_LOD03.gltf");
        match gltf::import(p) {
            Ok((doc, buffers, _imgs)) => {
                println!("PROBE import OK: meshes={} buffers={}", doc.meshes().count(), buffers.len());
                for m in doc.meshes() {
                    for prim in m.primitives() {
                        let r = prim.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
                        let pos: Vec<[f32;3]> = r.read_positions().map(|i| i.collect()).unwrap_or_default();
                        println!("PROBE prim {:?} positions={} first={:?}", m.name(), pos.len(), pos.first());
                        let nrm = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            r.read_normals().map(|i| i.collect::<Vec<[f32;3]>>())
                        }));
                        println!("PROBE normals: {:?}", nrm.as_ref().map(|o| o.as_ref().map(|v| (v.len(), v.first().copied()))));
                        let uv = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            r.read_tex_coords(0).map(|i| i.into_f32().collect::<Vec<[f32;2]>>())
                        }));
                        println!("PROBE uvs: {:?}", uv.as_ref().map(|o| o.as_ref().map(|v| (v.len(), v.first().copied()))));
                        break;
                    }
                    break;
                }
            }
            Err(e) => println!("PROBE import FAILED: {e}"),
        }
    }
}
