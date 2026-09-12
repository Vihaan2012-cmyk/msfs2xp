//! Record type identifiers.
//!
//! The identifiers themselves are stable across FS9 → MSFS 2024; what changes is
//! the body layout, and for most record types Asobo allocated a *new* id when
//! they changed the layout (e.g. runway `0x04` → `0x00CE`). That makes the id a
//! reliable first-order layout hint.

// Top level records
pub const REC_AIRPORT: u16 = 0x003C;
/// MSFS 2024 airport record: the 0x003C body plus 24 bytes, ident moved to a 64-bit field.
pub const REC_AIRPORT_MSFS2024: u16 = 0x0113;

// Airport sub-records
pub const AP_NAME: u16 = 0x0019;
pub const AP_TOWER_OBJ: u16 = 0x0066;
pub const AP_RUNWAY: u16 = 0x0004;
pub const AP_RUNWAY_P3D_V4: u16 = 0x003E;
pub const AP_RUNWAY_MSFS: u16 = 0x00CE;
pub const AP_WAYPOINT: u16 = 0x0022;
pub const AP_HELIPAD: u16 = 0x0026;
pub const AP_START: u16 = 0x0011;
pub const AP_COM: u16 = 0x0012;
pub const AP_DELETE_AIRPORT: u16 = 0x0033;
pub const AP_DELETE_AIRPORT_NAV: u16 = 0x00DB;
pub const AP_APRON_FIRST: u16 = 0x0037;
pub const AP_APRON_FIRST_P3D_V5: u16 = 0x00AF;
pub const AP_APRON_FIRST_MSFS: u16 = 0x00D3;
pub const AP_APRON_FIRST_MSFS_NEW: u16 = 0x00D0;
pub const AP_APRON_SECOND: u16 = 0x0030;
pub const AP_APRON_SECOND_P3D_V4: u16 = 0x0041;
pub const AP_APRON_SECOND_P3D_V5: u16 = 0x00B0;
pub const AP_APRON_EDGE_LIGHTS: u16 = 0x0031;
pub const AP_TAXI_POINT: u16 = 0x001A;
pub const AP_TAXI_POINT_P3D_V5: u16 = 0x00AC;
pub const AP_TAXI_PARKING: u16 = 0x003D;
pub const AP_TAXI_PARKING_P3D_V5: u16 = 0x00AD;
pub const AP_TAXI_PARKING_MSFS: u16 = 0x00E7;
pub const AP_TAXI_PARKING_FS9: u16 = 0x001B;
pub const AP_TAXI_PATH: u16 = 0x001C;
pub const AP_TAXI_PATH_P3D_V4: u16 = 0x0040;
pub const AP_TAXI_PATH_P3D_V5: u16 = 0x00AE;
pub const AP_TAXI_PATH_MSFS: u16 = 0x00D4;
pub const AP_TAXI_NAME: u16 = 0x001D;
pub const AP_JETWAY: u16 = 0x003A;
pub const AP_APPROACH: u16 = 0x0024;
pub const AP_APPROACH_MSFS: u16 = 0x00FA;
pub const AP_FENCE_BLAST: u16 = 0x0038;
pub const AP_FENCE_BOUNDARY: u16 = 0x0039;
pub const AP_UNKNOWN_003B: u16 = 0x003B;
pub const AP_MSFS_SID: u16 = 0x0042;
pub const AP_MSFS_STAR: u16 = 0x0048;
pub const AP_MSFS_LIGHT_SUPPORT: u16 = 0x0057;
pub const AP_MSFS_UNKNOWN_0058: u16 = 0x0058;
pub const AP_MSFS_UNKNOWN_0059: u16 = 0x0059;
pub const AP_MSFS_UNKNOWN_005A: u16 = 0x005A;
pub const AP_MSFS_UNKNOWN_005B: u16 = 0x005B;
pub const AP_MSFS_UNKNOWN_00CD: u16 = 0x00CD;
pub const AP_MSFS_PAINTED_LINE: u16 = 0x00CF;
pub const AP_MSFS_PAINTED_HATCHED_AREA: u16 = 0x00D8;
pub const AP_MSFS_TAXIWAY_SIGN: u16 = 0x00D9;
pub const AP_MSFS_PARKING_MFGR_NAME: u16 = 0x00DD;
pub const AP_MSFS_JETWAY: u16 = 0x00DE;
pub const AP_MSFS_PROJECTED_MESH: u16 = 0x00E8;
pub const AP_MSFS_GROUND_MERGING: u16 = 0x00E9;

// MSFS 2024 airport sub-records seen in iniBuilds packages. None carry data the
// conversion needs: 0x005C/0x005D are GUID references, 0x0102 names a WASM module.
pub const AP_MSFS2024_MATERIAL_REF: u16 = 0x005C;
pub const AP_MSFS2024_UNKNOWN_005D: u16 = 0x005D;
pub const AP_MSFS2024_UNKNOWN_006A: u16 = 0x006A;
pub const AP_MSFS2024_UNKNOWN_00FB: u16 = 0x00FB;
pub const AP_MSFS2024_UNKNOWN_00FF: u16 = 0x00FF;
pub const AP_MSFS2024_WASM: u16 = 0x0102;

// Runway sub-records
pub const RW_OFFSET_THRESHOLD_PRIM: u16 = 0x0005;
pub const RW_OFFSET_THRESHOLD_SEC: u16 = 0x0006;
pub const RW_BLAST_PAD_PRIM: u16 = 0x0007;
pub const RW_BLAST_PAD_SEC: u16 = 0x0008;
pub const RW_OVERRUN_PRIM: u16 = 0x0009;
pub const RW_OVERRUN_SEC: u16 = 0x000A;
pub const RW_OVERRUN_PRIM_MSFS: u16 = 0x0065;
pub const RW_OVERRUN_SEC_MSFS: u16 = 0x0066;
pub const RW_VASI_PRIM_LEFT: u16 = 0x000B;
pub const RW_VASI_PRIM_RIGHT: u16 = 0x000C;
pub const RW_VASI_SEC_LEFT: u16 = 0x000D;
pub const RW_VASI_SEC_RIGHT: u16 = 0x000E;
pub const RW_APP_LIGHTS_PRIM: u16 = 0x000F;
pub const RW_APP_LIGHTS_SEC: u16 = 0x0010;
pub const RW_APP_LIGHTS_PRIM_MSFS: u16 = 0x00DF;
pub const RW_APP_LIGHTS_SEC_MSFS: u16 = 0x00E0;
pub const RW_DEFORMATION_MSFS: u16 = 0x003E;
pub const RW_FACILITY_MATERIAL_MSFS: u16 = 0x00CB;

/// Is this a plausible airport sub-record id? Used by the layout probe.
pub fn is_airport_subrecord(id: u16) -> bool {
    matches!(
        id,
        AP_NAME
            | AP_TOWER_OBJ
            | AP_RUNWAY
            | AP_RUNWAY_P3D_V4
            | AP_RUNWAY_MSFS
            | AP_WAYPOINT
            | AP_HELIPAD
            | AP_START
            | AP_COM
            | AP_DELETE_AIRPORT
            | AP_DELETE_AIRPORT_NAV
            | AP_APRON_FIRST
            | AP_APRON_FIRST_P3D_V5
            | AP_APRON_FIRST_MSFS
            | AP_APRON_FIRST_MSFS_NEW
            | AP_APRON_SECOND
            | AP_APRON_SECOND_P3D_V4
            | AP_APRON_SECOND_P3D_V5
            | AP_APRON_EDGE_LIGHTS
            | AP_TAXI_POINT
            | AP_TAXI_POINT_P3D_V5
            | AP_TAXI_PARKING
            | AP_TAXI_PARKING_P3D_V5
            | AP_TAXI_PARKING_MSFS
            | AP_TAXI_PARKING_FS9
            | AP_TAXI_PATH
            | AP_TAXI_PATH_P3D_V4
            | AP_TAXI_PATH_P3D_V5
            | AP_TAXI_PATH_MSFS
            | AP_TAXI_NAME
            | AP_JETWAY
            | AP_APPROACH
            | AP_APPROACH_MSFS
            | AP_FENCE_BLAST
            | AP_FENCE_BOUNDARY
            | AP_UNKNOWN_003B
            | AP_MSFS_SID
            | AP_MSFS_STAR
            | AP_MSFS_LIGHT_SUPPORT
            | AP_MSFS_UNKNOWN_0058
            | AP_MSFS_UNKNOWN_0059
            | AP_MSFS_UNKNOWN_005A
            | AP_MSFS_UNKNOWN_005B
            | AP_MSFS_UNKNOWN_00CD
            | AP_MSFS_PAINTED_LINE
            | AP_MSFS_PAINTED_HATCHED_AREA
            | AP_MSFS_TAXIWAY_SIGN
            | AP_MSFS_PARKING_MFGR_NAME
            | AP_MSFS_JETWAY
            | AP_MSFS_PROJECTED_MESH
            | AP_MSFS_GROUND_MERGING
            | AP_MSFS2024_MATERIAL_REF
            | AP_MSFS2024_UNKNOWN_005D
            | AP_MSFS2024_UNKNOWN_006A
            | AP_MSFS2024_UNKNOWN_00FB
            | AP_MSFS2024_UNKNOWN_00FF
            | AP_MSFS2024_WASM
    )
}

/// Is this a plausible runway sub-record id?
pub fn is_runway_subrecord(id: u16) -> bool {
    matches!(
        id,
        RW_OFFSET_THRESHOLD_PRIM
            | RW_OFFSET_THRESHOLD_SEC
            | RW_BLAST_PAD_PRIM
            | RW_BLAST_PAD_SEC
            | RW_OVERRUN_PRIM
            | RW_OVERRUN_SEC
            | RW_OVERRUN_PRIM_MSFS
            | RW_OVERRUN_SEC_MSFS
            | RW_VASI_PRIM_LEFT
            | RW_VASI_PRIM_RIGHT
            | RW_VASI_SEC_LEFT
            | RW_VASI_SEC_RIGHT
            | RW_APP_LIGHTS_PRIM
            | RW_APP_LIGHTS_SEC
            | RW_APP_LIGHTS_PRIM_MSFS
            | RW_APP_LIGHTS_SEC_MSFS
            | RW_DEFORMATION_MSFS
            | RW_FACILITY_MATERIAL_MSFS
    )
}

/// Human-readable name for a record id, for `inspect` output.
pub fn airport_subrecord_name(id: u16) -> &'static str {
    match id {
        AP_NAME => "NAME",
        AP_TOWER_OBJ => "TOWER_OBJ",
        AP_RUNWAY => "RUNWAY",
        AP_RUNWAY_P3D_V4 => "RUNWAY_P3DV4",
        AP_RUNWAY_MSFS => "RUNWAY_MSFS",
        AP_WAYPOINT => "WAYPOINT",
        AP_HELIPAD => "HELIPAD",
        AP_START => "START",
        AP_COM => "COM",
        AP_DELETE_AIRPORT => "DELETE_AIRPORT",
        AP_DELETE_AIRPORT_NAV => "DELETE_AIRPORT_NAV",
        AP_APRON_FIRST => "APRON",
        AP_APRON_FIRST_P3D_V5 => "APRON_P3DV5",
        AP_APRON_FIRST_MSFS => "APRON_MSFS",
        AP_APRON_FIRST_MSFS_NEW => "APRON_MSFS_NEW",
        AP_APRON_SECOND => "APRON2",
        AP_APRON_SECOND_P3D_V4 => "APRON2_P3DV4",
        AP_APRON_SECOND_P3D_V5 => "APRON2_P3DV5",
        AP_APRON_EDGE_LIGHTS => "APRON_EDGE_LIGHTS",
        AP_TAXI_POINT => "TAXI_POINT",
        AP_TAXI_POINT_P3D_V5 => "TAXI_POINT_P3DV5",
        AP_TAXI_PARKING => "TAXI_PARKING",
        AP_TAXI_PARKING_P3D_V5 => "TAXI_PARKING_P3DV5",
        AP_TAXI_PARKING_MSFS => "TAXI_PARKING_MSFS",
        AP_TAXI_PARKING_FS9 => "TAXI_PARKING_FS9",
        AP_TAXI_PATH => "TAXI_PATH",
        AP_TAXI_PATH_P3D_V4 => "TAXI_PATH_P3DV4",
        AP_TAXI_PATH_P3D_V5 => "TAXI_PATH_P3DV5",
        AP_TAXI_PATH_MSFS => "TAXI_PATH_MSFS",
        AP_TAXI_NAME => "TAXI_NAME",
        AP_JETWAY => "JETWAY",
        AP_APPROACH => "APPROACH",
        AP_APPROACH_MSFS => "APPROACH_MSFS",
        AP_FENCE_BLAST => "FENCE_BLAST",
        AP_FENCE_BOUNDARY => "FENCE_BOUNDARY",
        AP_MSFS_SID => "SID",
        AP_MSFS_STAR => "STAR",
        AP_MSFS_LIGHT_SUPPORT => "LIGHT_SUPPORT",
        AP_MSFS_PAINTED_LINE => "PAINTED_LINE",
        AP_MSFS_PAINTED_HATCHED_AREA => "PAINTED_HATCHED_AREA",
        AP_MSFS_TAXIWAY_SIGN => "TAXIWAY_SIGN",
        AP_MSFS_PARKING_MFGR_NAME => "PARKING_MFGR_NAME",
        AP_MSFS_JETWAY => "JETWAY_MSFS",
        AP_MSFS_PROJECTED_MESH => "PROJECTED_MESH",
        AP_MSFS_GROUND_MERGING => "GROUND_MERGING",
        AP_MSFS2024_MATERIAL_REF => "MSFS2024_MATERIAL_REF",
        AP_MSFS2024_WASM => "MSFS2024_WASM",
        AP_MSFS2024_UNKNOWN_005D | AP_MSFS2024_UNKNOWN_006A | AP_MSFS2024_UNKNOWN_00FB | AP_MSFS2024_UNKNOWN_00FF => {
            "MSFS2024_UNUSED"
        }
        _ => "UNKNOWN",
    }
}

/// Human-readable name for a runway sub-record id.
pub fn runway_subrecord_name(id: u16) -> &'static str {
    match id {
        RW_OFFSET_THRESHOLD_PRIM => "OFFSET_THRESHOLD_PRIMARY",
        RW_OFFSET_THRESHOLD_SEC => "OFFSET_THRESHOLD_SECONDARY",
        RW_BLAST_PAD_PRIM => "BLAST_PAD_PRIMARY",
        RW_BLAST_PAD_SEC => "BLAST_PAD_SECONDARY",
        RW_OVERRUN_PRIM | RW_OVERRUN_PRIM_MSFS => "OVERRUN_PRIMARY",
        RW_OVERRUN_SEC | RW_OVERRUN_SEC_MSFS => "OVERRUN_SECONDARY",
        RW_VASI_PRIM_LEFT => "VASI_PRIMARY_LEFT",
        RW_VASI_PRIM_RIGHT => "VASI_PRIMARY_RIGHT",
        RW_VASI_SEC_LEFT => "VASI_SECONDARY_LEFT",
        RW_VASI_SEC_RIGHT => "VASI_SECONDARY_RIGHT",
        RW_APP_LIGHTS_PRIM | RW_APP_LIGHTS_PRIM_MSFS => "APPROACH_LIGHTS_PRIMARY",
        RW_APP_LIGHTS_SEC | RW_APP_LIGHTS_SEC_MSFS => "APPROACH_LIGHTS_SECONDARY",
        RW_DEFORMATION_MSFS => "RUNWAY_DEFORMATION",
        RW_FACILITY_MATERIAL_MSFS => "FACILITY_MATERIAL",
        _ => "UNKNOWN",
    }
}
