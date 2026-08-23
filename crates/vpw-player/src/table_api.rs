//! Bridge to JS for reading `.vpx` files from the menu.
//!
//! This crate is the app's only wasm<->JS boundary, so the bindings for the
//! table library live here even though the logic is in `vpw-table`.

use vpw_table::RomRequirement;
use wasm_bindgen::prelude::*;

/// Summary of a table, exactly as the menu consumes it.
#[wasm_bindgen]
pub struct TableInfo {
    inner: vpw_table::TableSummary,
}

#[wasm_bindgen]
impl TableInfo {
    #[wasm_bindgen(getter, js_name = tableName)]
    pub fn table_name(&self) -> Option<String> {
        self.inner.table_name.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn author(&self) -> Option<String> {
        self.inner.author.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn version(&self) -> Option<String> {
        self.inner.version.clone()
    }

    #[wasm_bindgen(getter, js_name = releaseDate)]
    pub fn release_date(&self) -> Option<String> {
        self.inner.release_date.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn description(&self) -> Option<String> {
        self.inner.description.clone()
    }

    /// Version of the VPX format, e.g. 1080 for VPX 10.8.
    #[wasm_bindgen(getter, js_name = fileVersion)]
    pub fn file_version(&self) -> u32 {
        self.inner.file_version
    }

    #[wasm_bindgen(getter, js_name = gameitemCount)]
    pub fn gameitem_count(&self) -> usize {
        self.inner.gameitem_count
    }

    #[wasm_bindgen(getter, js_name = imageCount)]
    pub fn image_count(&self) -> usize {
        self.inner.image_count
    }

    #[wasm_bindgen(getter, js_name = soundCount)]
    pub fn sound_count(&self) -> usize {
        self.inner.sound_count
    }

    #[wasm_bindgen(getter, js_name = scriptLen)]
    pub fn script_len(&self) -> usize {
        self.inner.script_len
    }

    /// `"none"` | `"required"` | `"unknown"`.
    ///
    /// `"unknown"` means the table uses VPinMAME but builds the ROM name at
    /// runtime, so we have to ask the user for it.
    #[wasm_bindgen(getter, js_name = romStatus)]
    pub fn rom_status(&self) -> String {
        match self.inner.rom {
            RomRequirement::NotNeeded => "none",
            RomRequirement::Required { .. } => "required",
            RomRequirement::RequiredUnknown => "unknown",
        }
        .to_string()
    }

    /// Name of the PinMAME set, e.g. `f14_l1`.
    #[wasm_bindgen(getter, js_name = romName)]
    pub fn rom_name(&self) -> Option<String> {
        match &self.inner.rom {
            RomRequirement::Required { game_name, .. } => Some(game_name.clone()),
            _ => None,
        }
    }

    /// The file that has to be obtained, e.g. `f14_l1.zip`.
    #[wasm_bindgen(getter, js_name = romZip)]
    pub fn rom_zip(&self) -> Option<String> {
        self.inner.rom.zip_file_name()
    }

    /// Other ROM versions the script mentions.
    #[wasm_bindgen(getter, js_name = romAlternates)]
    pub fn rom_alternates(&self) -> Vec<String> {
        match &self.inner.rom {
            RomRequirement::Required { alternates, .. } => alternates.clone(),
            _ => Vec::new(),
        }
    }

    /// Screenshot embedded in the `.vpx`, if it carries one. Raw image bytes.
    #[wasm_bindgen(getter)]
    pub fn screenshot(&self) -> Option<Vec<u8>> {
        self.inner.screenshot.clone()
    }
}

/// Reads a `.vpx` and returns its summary for the menu.
#[wasm_bindgen(js_name = readTable)]
pub fn read_table(bytes: &[u8]) -> Result<TableInfo, JsError> {
    let inner = vpw_table::summarize(bytes).map_err(|e| JsError::new(&e.to_string()))?;
    Ok(TableInfo { inner })
}
