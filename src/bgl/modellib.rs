//! Model libraries: where the 3D buildings live.
//!
//! An MSFS scenery package ships its models compiled into a "modellib" BGL,
//! often close to a gigabyte. The file holds a MODEL_DATA section (0x2B) whose
//! subsection begins with an index of `GUID, u32 offset, u32 size` entries; the
//! offsets are relative to the start of that subsection's data. Each entry
//! points at a RIFF container of form `GLTF` holding:
//!
//! - `GXML`: the ModelInfo XML, giving the model's name and its LOD list,
//!   most detailed first;
//! - `GLBD`: one `GLBZ` chunk per LOD, each a `u32` inflated length followed by a
//!   Zstandard frame that inflates to a standard binary glTF (GLB v2) file.
//!
//! Placed scenery objects name their model by GUID, so this module is the bridge
//! between "put object X here" and the geometry of X. Libraries are read with
//! seeks rather than loaded whole, because several of them would not fit in
//! memory at once.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::file::{HEADER_SIZE, MAGIC1, MAGIC2};
use super::guid::Guid;

pub const SECTION_MODEL_DATA: u32 = 0x2B;
const INDEX_ENTRY_SIZE: usize = 24;
/// A real LOD inflates to a few megabytes. This only stops a corrupt length
/// field from exhausting memory.
const MAX_GLB_SIZE: usize = 512 * 1024 * 1024;

#[derive(thiserror::Error, Debug)]
pub enum ModelLibError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a BGL file")]
    NotBgl,
    #[error("the file has no model data section")]
    NoModelData,
    #[error("model {0} is not in this library")]
    UnknownModel(Guid),
    #[error("malformed model container: {0}")]
    Malformed(String),
    #[error("could not decompress LOD: {0}")]
    Decompress(String),
}

/// Where one model's container sits in the library file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexEntry {
    pub guid: Guid,
    pub offset: u64,
    pub size: u32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Lod {
    /// The source glTF file name the LOD was built from.
    pub file: String,
    /// Screen-size threshold below which MSFS switches to the next LOD.
    pub min_size: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModelInfo {
    pub guid: Option<Guid>,
    pub name: String,
    pub lods: Vec<Lod>,
}

/// One model's container, borrowed from its bytes.
#[derive(Debug, Clone)]
pub struct ModelBlob<'a> {
    pub xml: Option<&'a str>,
    /// Each LOD chunk as `(chunk id, body)`, in file order (most detailed first).
    pub lods: Vec<([u8; 4], &'a [u8])>,
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at.checked_add(4)?)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// Walk RIFF-style chunks (`4cc`, `u32 size`, body padded to an even length),
/// stopping cleanly at the first chunk that would run past the buffer.
fn chunks(buf: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + 8 <= buf.len() {
        let id = [buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]];
        let Some(size) = le32(buf, pos + 4).map(|s| s as usize) else {
            break;
        };
        let start = pos + 8;
        let end = match start.checked_add(size) {
            Some(e) if e <= buf.len() => e,
            _ => break,
        };
        out.push((id, &buf[start..end]));
        pos = end + (size & 1);
    }
    out
}

fn trim_nul(b: &[u8]) -> &[u8] {
    let end = b.iter().rposition(|&c| c != 0).map_or(0, |p| p + 1);
    &b[..end]
}

/// Split a model container into its XML and LOD chunks.
pub fn parse_model_blob(blob: &[u8]) -> Result<ModelBlob<'_>, ModelLibError> {
    if blob.len() < 12 || &blob[0..4] != b"RIFF" {
        return Err(ModelLibError::Malformed("missing RIFF header".into()));
    }
    if &blob[8..12] != b"GLTF" {
        return Err(ModelLibError::Malformed(format!(
            "unexpected RIFF form {:?}",
            String::from_utf8_lossy(&blob[8..12])
        )));
    }
    let declared = le32(blob, 4).unwrap_or(0) as usize;
    let body_end = 8usize.saturating_add(declared).min(blob.len());
    let mut xml = None;
    let mut lods = Vec::new();
    for (id, body) in chunks(&blob[12..body_end]) {
        match &id {
            b"GXML" => xml = std::str::from_utf8(trim_nul(body)).ok(),
            b"GLBD" => lods.extend(chunks(body)),
            _ => {}
        }
    }
    Ok(ModelBlob { xml, lods })
}

/// Turn one LOD chunk into the bytes of a GLB file.
pub fn decode_lod(id: &[u8; 4], body: &[u8]) -> Result<Vec<u8>, ModelLibError> {
    if id == b"GLBZ" {
        let declared = le32(body, 0).ok_or_else(|| ModelLibError::Decompress("GLBZ chunk too short".into()))? as usize;
        if declared > MAX_GLB_SIZE {
            return Err(ModelLibError::Decompress(format!(
                "declared size {declared} is implausibly large"
            )));
        }
        let mut decoder = ruzstd::decoding::StreamingDecoder::new(&body[4..])
            .map_err(|e| ModelLibError::Decompress(e.to_string()))?;
        let mut out = Vec::with_capacity(declared);
        // Read one byte beyond the declared size so an over-long stream is caught.
        (&mut decoder)
            .take(declared as u64 + 1)
            .read_to_end(&mut out)
            .map_err(|e| ModelLibError::Decompress(e.to_string()))?;
        if out.len() != declared {
            return Err(ModelLibError::Decompress(format!(
                "inflated to {} bytes but the header said {declared}",
                out.len()
            )));
        }
        return Ok(out);
    }
    // An uncompressed GLB under any chunk id is usable as it stands.
    if body.starts_with(b"glTF") {
        return Ok(body.to_vec());
    }
    Err(ModelLibError::Malformed(format!(
        "unknown LOD chunk {:?}",
        String::from_utf8_lossy(id)
    )))
}

/// Read the name and LOD list out of the ModelInfo XML.
pub fn parse_model_info(xml: &str) -> ModelInfo {
    let mut info = ModelInfo::default();
    let Ok(doc) = roxmltree::Document::parse(xml) else {
        return info;
    };
    for node in doc.descendants() {
        match node.tag_name().name() {
            "ModelInfo" => {
                info.name = node.attribute("name").unwrap_or_default().to_string();
                info.guid = node.attribute("guid").and_then(|g| g.parse().ok());
            }
            "LOD" => info.lods.push(Lod {
                file: node.attribute("ModelFile").unwrap_or_default().to_string(),
                min_size: node.attribute("MinSize").and_then(|v| v.parse().ok()).unwrap_or(0.0),
            }),
            _ => {}
        }
    }
    info
}

/// An open model library file.
#[derive(Debug)]
pub struct ModelLibrary {
    path: PathBuf,
    entries: HashMap<Guid, IndexEntry>,
    order: Vec<Guid>,
}

impl ModelLibrary {
    /// Read the model index. Only the headers and the index are read here.
    pub fn open(path: &Path) -> Result<Self, ModelLibError> {
        let mut f = File::open(path)?;
        let file_len = f.metadata()?.len();
        let mut head = [0u8; HEADER_SIZE];
        f.read_exact(&mut head).map_err(|_| ModelLibError::NotBgl)?;
        if le32(&head, 0) != Some(MAGIC1) || le32(&head, 16) != Some(MAGIC2) {
            return Err(ModelLibError::NotBgl);
        }
        let count = le32(&head, 0x14).unwrap_or(0) as usize;
        if count > 4096 {
            return Err(ModelLibError::Malformed(format!("{count} sections")));
        }
        let mut table = vec![0u8; count * 20];
        f.read_exact(&mut table)?;

        let mut entries = HashMap::new();
        let mut order = Vec::new();
        for t in table.chunks_exact(20) {
            if le32(t, 0) != Some(SECTION_MODEL_DATA) {
                continue;
            }
            let flag = le32(t, 4).unwrap_or(0);
            let sub_count = le32(t, 8).unwrap_or(0) as usize;
            let sub_offset = le32(t, 12).unwrap_or(0) as u64;
            let sub_size = if flag & 0x10000 != 0 { 20 } else { 16 };
            if sub_count > 65_536 || sub_offset + (sub_count * sub_size) as u64 > file_len {
                return Err(ModelLibError::Malformed("model section table out of range".into()));
            }
            let mut subs = vec![0u8; sub_count * sub_size];
            f.seek(SeekFrom::Start(sub_offset))?;
            f.read_exact(&mut subs)?;
            for s in subs.chunks_exact(sub_size) {
                let records = le32(s, 4).unwrap_or(0) as u64;
                let data_offset = le32(s, 8).unwrap_or(0) as u64;
                if data_offset + records * INDEX_ENTRY_SIZE as u64 > file_len {
                    return Err(ModelLibError::Malformed("model index out of range".into()));
                }
                let mut index = vec![0u8; records as usize * INDEX_ENTRY_SIZE];
                f.seek(SeekFrom::Start(data_offset))?;
                f.read_exact(&mut index)?;
                for e in index.chunks_exact(INDEX_ENTRY_SIZE) {
                    let Some(guid) = Guid::from_slice(e) else { continue };
                    let offset = data_offset + le32(e, 16).unwrap_or(0) as u64;
                    let size = le32(e, 20).unwrap_or(0);
                    // Skip entries that point outside the file rather than failing
                    // the whole library over one bad record.
                    if size == 0 || offset + size as u64 > file_len {
                        continue;
                    }
                    if entries.insert(guid, IndexEntry { guid, offset, size }).is_none() {
                        order.push(guid);
                    }
                }
            }
        }
        if entries.is_empty() {
            return Err(ModelLibError::NoModelData);
        }
        Ok(ModelLibrary {
            path: path.to_path_buf(),
            entries,
            order,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn contains(&self, guid: &Guid) -> bool {
        self.entries.contains_key(guid)
    }

    /// Model GUIDs in the order the library lists them.
    pub fn guids(&self) -> &[Guid] {
        &self.order
    }

    fn entry(&self, guid: &Guid) -> Result<&IndexEntry, ModelLibError> {
        self.entries.get(guid).ok_or(ModelLibError::UnknownModel(*guid))
    }

    fn read_blob(&self, entry: &IndexEntry) -> Result<Vec<u8>, ModelLibError> {
        let mut f = File::open(&self.path)?;
        f.seek(SeekFrom::Start(entry.offset))?;
        let mut blob = vec![0u8; entry.size as usize];
        f.read_exact(&mut blob)?;
        Ok(blob)
    }

    /// The model's name and LOD list.
    pub fn info(&self, guid: &Guid) -> Result<ModelInfo, ModelLibError> {
        let blob = self.read_blob(self.entry(guid)?)?;
        let parsed = parse_model_blob(&blob)?;
        let mut info = parsed.xml.map(parse_model_info).unwrap_or_default();
        if info.guid.is_none() {
            info.guid = Some(*guid);
        }
        // Keep the LOD list the same length as the geometry actually present.
        info.lods.resize(parsed.lods.len(), Lod::default());
        Ok(info)
    }

    /// The GLB bytes of one LOD, 0 being the most detailed.
    pub fn load_lod(&self, guid: &Guid, lod: usize) -> Result<Vec<u8>, ModelLibError> {
        let blob = self.read_blob(self.entry(guid)?)?;
        let parsed = parse_model_blob(&blob)?;
        let (id, body) = parsed
            .lods
            .get(lod)
            .ok_or_else(|| ModelLibError::Malformed(format!("model {guid} has no LOD {lod}")))?;
        decode_lod(id, body)
    }
}

/// Every model library in a package, searched together.
#[derive(Debug, Default)]
pub struct ModelCatalog {
    libraries: Vec<ModelLibrary>,
    by_guid: HashMap<Guid, usize>,
}

impl ModelCatalog {
    /// Add a library. A model defined in more than one keeps its first definition.
    pub fn add(&mut self, lib: ModelLibrary) {
        let idx = self.libraries.len();
        for g in lib.guids() {
            self.by_guid.entry(*g).or_insert(idx);
        }
        self.libraries.push(lib);
    }

    pub fn find(&self, guid: &Guid) -> Option<&ModelLibrary> {
        self.by_guid.get(guid).map(|&i| &self.libraries[i])
    }

    pub fn len(&self) -> usize {
        self.by_guid.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_guid.is_empty()
    }

    pub fn libraries(&self) -> &[ModelLibrary] {
        &self.libraries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Zstandard frame holding `payload` as one raw (stored) block. Built by
    /// hand so the tests need no encoder.
    fn raw_zstd_frame(payload: &[u8]) -> Vec<u8> {
        assert!(payload.len() <= 255, "test helper uses a one-byte content size");
        let mut out = vec![0x28, 0xB5, 0x2F, 0xFD];
        out.push(0x20); // single segment, one-byte frame content size, no checksum
        out.push(payload.len() as u8);
        let header = 1u32 | ((payload.len() as u32) << 3); // last block, raw type
        out.extend_from_slice(&header.to_le_bytes()[..3]);
        out.extend_from_slice(payload);
        out
    }

    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    fn fake_glb(tag: u8) -> Vec<u8> {
        let mut g = b"glTF".to_vec();
        g.extend_from_slice(&2u32.to_le_bytes());
        g.extend_from_slice(&16u32.to_le_bytes());
        g.extend_from_slice(&[tag; 4]);
        g
    }

    fn glbz(glb: &[u8]) -> Vec<u8> {
        let mut body = (glb.len() as u32).to_le_bytes().to_vec();
        body.extend_from_slice(&raw_zstd_frame(glb));
        chunk(b"GLBZ", &body)
    }

    fn model_blob(name: &str, guid: &str, lods: &[Vec<u8>]) -> Vec<u8> {
        let xml = format!(
            "<ModelInfo guid=\"{guid}\" version=\"1.1\" name=\"{name}\"><LODS>{}</LODS></ModelInfo>\0\0",
            (0..lods.len())
                .map(|i| format!("<LOD ModelFile=\"{name}_LOD{i}.gltf\" MinSize=\"{}\"/>", 3 - i))
                .collect::<String>()
        );
        let mut glbd = Vec::new();
        for l in lods {
            glbd.extend_from_slice(&glbz(l));
        }
        let mut form = b"GLTF".to_vec();
        form.extend_from_slice(&chunk(b"GXML", xml.as_bytes()));
        form.extend_from_slice(&chunk(b"GLBD", &glbd));
        chunk(b"RIFF", &form)
    }

    const GUID_TEXT: &str = "{f9ce0800-4f38-4d70-a87a-38bc99899e0b}";

    #[test]
    fn parses_a_container_and_inflates_each_lod() {
        let blob = model_blob("OMDB_GATE_B12", GUID_TEXT, &[fake_glb(1), fake_glb(2)]);
        let parsed = parse_model_blob(&blob).unwrap();
        assert_eq!(parsed.lods.len(), 2);
        let info = parse_model_info(parsed.xml.unwrap());
        assert_eq!(info.name, "OMDB_GATE_B12");
        assert_eq!(info.guid, Some(GUID_TEXT.parse().unwrap()));
        assert_eq!(info.lods.len(), 2);
        assert_eq!(info.lods[1].file, "OMDB_GATE_B12_LOD1.gltf");
        let (id, body) = &parsed.lods[1];
        assert_eq!(decode_lod(id, body).unwrap(), fake_glb(2));
    }

    #[test]
    fn nul_padded_xml_is_trimmed() {
        let blob = model_blob("X", GUID_TEXT, &[fake_glb(0)]);
        let parsed = parse_model_blob(&blob).unwrap();
        assert!(!parsed.xml.unwrap().ends_with('\0'));
    }

    #[test]
    fn a_length_mismatch_is_an_error() {
        let glb = fake_glb(7);
        let mut body = ((glb.len() + 3) as u32).to_le_bytes().to_vec();
        body.extend_from_slice(&raw_zstd_frame(&glb));
        assert!(matches!(decode_lod(b"GLBZ", &body), Err(ModelLibError::Decompress(_))));
    }

    #[test]
    fn garbage_is_rejected_not_panicked_on() {
        assert!(parse_model_blob(b"not a riff").is_err());
        assert!(parse_model_blob(b"RIFF\x04\x00\x00\x00WAVE").is_err());
        assert!(decode_lod(b"GLBZ", &[1, 2]).is_err());
        assert!(decode_lod(b"GLBZ", &[4, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]).is_err());
        assert!(decode_lod(b"ABCD", b"nonsense").is_err());
        // A container whose chunk sizes overrun the buffer yields no LODs.
        let mut blob = model_blob("X", GUID_TEXT, &[fake_glb(0)]);
        blob.truncate(blob.len() - 5);
        assert!(parse_model_blob(&blob).unwrap().lods.is_empty());
    }

    #[test]
    fn uncompressed_glb_passes_through() {
        let glb = fake_glb(9);
        assert_eq!(decode_lod(b"GLB\0", &glb).unwrap(), glb);
    }

    /// Write a minimal library file: header, one MODEL_DATA section, one
    /// subsection whose data is the index followed by the model containers.
    fn write_library(models: &[(Guid, Vec<u8>)]) -> tempfile::NamedTempFile {
        let section_table = HEADER_SIZE;
        let sub_table = section_table + 20;
        let data = sub_table + 16;
        let index_len = models.len() * INDEX_ENTRY_SIZE;
        let mut index = Vec::new();
        let mut blobs = Vec::new();
        for (g, blob) in models {
            index.extend_from_slice(&g.0);
            index.extend_from_slice(&((index_len + blobs.len()) as u32).to_le_bytes());
            index.extend_from_slice(&(blob.len() as u32).to_le_bytes());
            blobs.extend_from_slice(blob);
        }
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC1.to_le_bytes());
        out.extend_from_slice(&(HEADER_SIZE as u32).to_le_bytes());
        out.extend_from_slice(&[0u8; 8]);
        out.extend_from_slice(&MAGIC2.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&[0u8; 32]);
        for v in [SECTION_MODEL_DATA, 0, 1, sub_table as u32, 16] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for v in [0u32, models.len() as u32, data as u32, (index_len + blobs.len()) as u32] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&index);
        out.extend_from_slice(&blobs);
        let mut file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut file, &out).unwrap();
        file
    }

    #[test]
    fn opens_a_library_file_and_loads_by_guid() {
        let a: Guid = GUID_TEXT.parse().unwrap();
        let b: Guid = "{380f3e00-532b-4533-b6de-513aa4606794}".parse().unwrap();
        let file = write_library(&[
            (a, model_blob("GATE_B12", GUID_TEXT, &[fake_glb(1), fake_glb(2)])),
            (
                b,
                model_blob("GATE_G11", "{380f3e00-532b-4533-b6de-513aa4606794}", &[fake_glb(3)]),
            ),
        ]);
        let lib = ModelLibrary::open(file.path()).unwrap();
        assert_eq!(lib.len(), 2);
        assert_eq!(lib.guids(), &[a, b]);
        assert_eq!(lib.info(&b).unwrap().name, "GATE_G11");
        assert_eq!(lib.load_lod(&a, 1).unwrap(), fake_glb(2));
        assert_eq!(lib.load_lod(&b, 0).unwrap(), fake_glb(3));
        assert!(matches!(lib.load_lod(&b, 5), Err(ModelLibError::Malformed(_))));
        assert!(matches!(lib.info(&Guid::NIL), Err(ModelLibError::UnknownModel(_))));

        let mut catalog = ModelCatalog::default();
        catalog.add(lib);
        assert!(catalog.find(&a).is_some());
        assert!(catalog.find(&Guid::NIL).is_none());
        assert_eq!(catalog.len(), 2);
    }

    #[test]
    fn a_file_without_models_is_reported() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut file, b"definitely not a bgl file at all, just text").unwrap();
        assert!(matches!(ModelLibrary::open(file.path()), Err(ModelLibError::NotBgl)));
    }

    /// Run against a real library: `MSFS2XP_MODELLIB=<path to a modellib .BGL>`.
    #[test]
    #[ignore]
    fn real_library_inflates_every_lod0() {
        let Ok(path) = std::env::var("MSFS2XP_MODELLIB") else {
            return;
        };
        let lib = ModelLibrary::open(Path::new(&path)).unwrap();
        let mut ok = 0;
        for g in lib.guids() {
            let glb = lib.load_lod(g, 0).unwrap_or_else(|e| panic!("{g}: {e}"));
            assert!(glb.starts_with(b"glTF"), "{g} did not inflate to a GLB");
            ok += 1;
        }
        eprintln!("inflated LOD0 of {ok} models from {path}");
    }
}
