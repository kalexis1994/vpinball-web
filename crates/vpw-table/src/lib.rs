//! Reading `.vpx` tables: metadata for the menu and ROM detection.
//!
//! Parsing the container (OLE Compound File + BIFF records) is done by the
//! [`vpin`] crate, so all we do here is build the summary the UI consumes.

pub mod animation;
pub mod backbox;
pub mod ball;
pub mod builtin;
pub mod controls;
pub mod dragpoint;
pub mod flipper;
pub mod geometry;
pub mod light;
pub mod meshes;
pub mod physics;
pub mod plunger;
pub mod ramp;
pub mod rom;
pub mod rubber;
pub mod sound;
pub mod surface;
pub mod triangulate;
pub mod trigger;

pub use geometry::{Scene, extract};

pub use rom::RomRequirement;

/// Everything the menu needs to know about a table without fully loading it.
#[derive(Debug, Clone)]
pub struct TableSummary {
    pub table_name: Option<String>,
    pub author: Option<String>,
    pub version: Option<String>,
    pub release_date: Option<String>,
    pub description: Option<String>,
    /// Screenshot embedded in the `.vpx`, if the table ships one. Works as a
    /// thumbnail.
    pub screenshot: Option<Vec<u8>>,
    pub rom: RomRequirement,
    /// File format version (e.g. 1080 = VPX 10.8).
    pub file_version: u32,
    pub gameitem_count: usize,
    pub image_count: usize,
    pub sound_count: usize,
    /// Length of the VBScript in bytes. Useful as a complexity signal.
    pub script_len: usize,
}

#[derive(Debug)]
pub struct ReadError(String);

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "could not read the .vpx: {}", self.0)
    }
}

impl std::error::Error for ReadError {}

/// Reads a `.vpx` in memory and returns its summary.
pub fn summarize(bytes: &[u8]) -> Result<TableSummary, ReadError> {
    let vpx = vpin::vpx::from_bytes(bytes).map_err(|e| ReadError(e.to_string()))?;

    let script = vpx.gamedata.code.string.as_str();
    let info = vpx.info;

    Ok(TableSummary {
        table_name: non_empty(info.table_name),
        author: non_empty(info.author_name),
        version: non_empty(info.table_version),
        release_date: non_empty(info.release_date),
        description: non_empty(info.table_description),
        screenshot: info.screenshot.filter(|s| !s.is_empty()),
        rom: rom::detect(script),
        file_version: vpx.version.u32(),
        gameitem_count: vpx.gameitems.len(),
        image_count: vpx.images.len(),
        sound_count: vpx.sounds.len(),
        script_len: script.len(),
    })
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}
