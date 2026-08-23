//! Detecting the ROM a table needs.
//!
//! The VPX format does **not** store the ROM name in any field: it is defined by
//! the table's VBScript, normally as `Const cGameName = "f14_l1"`, and the
//! PinMAME plugin reads it at runtime through the `GameName` property (see
//! `plugins/pinmame/PinMAMEPlugin.cpp:140` in the original repo).
//!
//! So the only way is to parse the script. We do it by hand rather than with
//! `regex` so as not to add ~1 MB to the wasm bundle.

/// What the table needs as far as ROMs go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RomRequirement {
    /// No reference to VPinMAME: it is an original table, it plays as-is.
    NotNeeded,
    /// It uses VPinMAME and we managed to read the game name.
    Required {
        /// Name of the PinMAME set, e.g. `f14_l1`.
        game_name: String,
        /// Other names that show up in the script (alternate ROM versions the
        /// table supports).
        alternates: Vec<String>,
    },
    /// It uses VPinMAME but we could not extract the name. It has to be asked
    /// of the user.
    RequiredUnknown,
}

impl RomRequirement {
    /// Name of the file you have to get hold of, e.g. `f14_l1.zip`.
    pub fn zip_file_name(&self) -> Option<String> {
        match self {
            Self::Required { game_name, .. } => Some(format!("{game_name}.zip")),
            _ => None,
        }
    }
}

/// Markers that the table talks to PinMAME.
///
/// Looking only for `VPinMAME` is not enough: modern tables load the controller
/// indirectly with `LoadVPM "01560000", "S11.VBS", 3.26`, which in turn does
/// `GetTextFile("controller.vbs")`. Williams' F-14, for example, does not
/// contain the string `VPinMAME` anywhere in its script.
/// Careful about loosening this list: `controller.vbs` does **not** work as a
/// marker. Original tables load it too, but for DOF (hardware feedback), not for
/// PinMAME. The VPX demo table is exactly that case.
const PINMAME_MARKERS: [&str; 5] = ["vpinmame", "loadvpm", "core.vbs", "vpminit", ".gamename"];

/// Analyses a table's VBScript and determines which ROM it needs.
pub fn detect(script: &str) -> RomRequirement {
    let code = strip_comments(script);
    let lower = code.to_ascii_lowercase();

    let mut names = Vec::new();
    for line in code.lines() {
        // `cGameName` is the near-universal convention; `.GameName = "..."`
        // shows up in tables that assign the ROM straight to the controller.
        for ident in ["cgamename", "gamename"] {
            if let Some(value) = assigned_string(line, ident)
                && !value.is_empty()
                && !names.contains(&value)
            {
                names.push(value);
            }
        }
    }

    // `cGameName` on its own is NOT enough: DOF uses the same constant as a
    // table identifier. "Nudge Test and Calibration" declares
    // `Const cGameName = "dof_test"` and never touches PinMAME. A marker that
    // the table really does talk to the emulator is needed as well.
    if !PINMAME_MARKERS.iter().any(|m| lower.contains(m)) {
        return RomRequirement::NotNeeded;
    }

    match names.split_first() {
        Some((first, rest)) => RomRequirement::Required {
            game_name: first.clone(),
            alternates: rest.to_vec(),
        },
        None => RomRequirement::RequiredUnknown,
    }
}

/// Strips VBScript comments (`'` to end of line), respecting quotes. Without
/// this we match commented-out ROMs, and there are plenty: tables usually leave
/// several versions listed and only one active.
fn strip_comments(script: &str) -> String {
    let mut out = String::with_capacity(script.len());
    for line in script.lines() {
        let mut in_string = false;
        let mut end = line.len();
        for (i, c) in line.char_indices() {
            match c {
                '"' => in_string = !in_string,
                '\'' if !in_string => {
                    end = i;
                    break;
                }
                _ => {}
            }
        }
        out.push_str(&line[..end]);
        out.push('\n');
    }
    out
}

/// Looks for `<ident> = "value"` in a line. VBScript is case-insensitive, and
/// `ident` has to arrive already in lower case.
///
/// It matches `Const cGameName = "x"` as well as `cGameName="x"` and
/// `.GameName = "x"`, but it requires `ident` to be a whole word so as not to
/// confuse `GameName` with `cGameName` nor with `MyGameNameFoo`.
fn assigned_string(line: &str, ident: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let mut from = 0;

    while let Some(rel) = lower[from..].find(ident) {
        let start = from + rel;
        let end = start + ident.len();
        from = end;

        let before_ok = line[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_');
        let after_ok = line[end..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_');
        if !before_ok || !after_ok {
            continue;
        }

        let rest = line[end..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('"') else {
            continue;
        };
        if let Some(close) = rest.find('"') {
            return Some(rest[..close].to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_original_table_needs_no_rom() {
        let script = "Sub Table1_Init\n  msgbox \"hello\"\nEnd Sub";
        assert_eq!(detect(script), RomRequirement::NotNeeded);
    }

    #[test]
    fn it_detects_const_cgamename() {
        let script = r#"
            Const cGameName = "f14_l1"
            Set Controller = CreateObject("VPinMAME.Controller")
            Controller.GameName = cGameName
        "#;
        let rom = detect(script);
        assert_eq!(
            rom,
            RomRequirement::Required {
                game_name: "f14_l1".into(),
                alternates: vec![]
            }
        );
        assert_eq!(rom.zip_file_name().unwrap(), "f14_l1.zip");
    }

    #[test]
    fn it_ignores_commented_out_roms() {
        // The usual pattern: several versions listed, only one active.
        let script = r#"
            'Const cGameName = "taf_l1"
            Const cGameName = "taf_l7"
            ' Const cGameName = "taf_p2"
            Set c = CreateObject("VPinMAME.Controller")
        "#;
        match detect(script) {
            RomRequirement::Required {
                game_name,
                alternates,
            } => {
                assert_eq!(game_name, "taf_l7");
                assert!(
                    alternates.is_empty(),
                    "alternates left over: {alternates:?}"
                );
            }
            other => panic!("expected Required, got {other:?}"),
        }
    }

    #[test]
    fn it_does_not_mistake_an_apostrophe_inside_a_string() {
        let script = r#"
            Const cGameName = "mm_109c" ' Medieval Madness
            msgbox "it's fine"
            Set c = CreateObject("VPinMAME.Controller")
        "#;
        match detect(script) {
            RomRequirement::Required { game_name, .. } => assert_eq!(game_name, "mm_109c"),
            other => panic!("expected Required, got {other:?}"),
        }
    }

    #[test]
    fn it_collects_alternate_versions() {
        let script = r#"
            Const cGameName = "afm_113b"
            If UseOldRom Then cGameName2 = "afm_11"
            .GameName = "afm_95"
            Set c = CreateObject("VPinMAME.Controller")
        "#;
        match detect(script) {
            RomRequirement::Required {
                game_name,
                alternates,
            } => {
                assert_eq!(game_name, "afm_113b");
                // `cGameName2` does not match (not a whole word), `.GameName` does.
                assert_eq!(alternates, vec!["afm_95".to_string()]);
            }
            other => panic!("expected Required, got {other:?}"),
        }
    }

    #[test]
    fn it_detects_the_loadvpm_pattern_without_mentioning_vpinmame() {
        // A real case: F-14 Tomcat (Williams 1987). Its script never contains
        // the string "VPinMAME"; it loads the controller via LoadVPM.
        let script = r#"
            Const cGameName="f14_l1"
            LoadVPM "01560000", "S11.VBS", 3.26
            Sub LoadVPM(VPMver, VBSfile, VBSver)
              ExecuteGlobal GetTextFile(VBSfile)
            End Sub
        "#;
        match detect(script) {
            RomRequirement::Required { game_name, .. } => {
                assert_eq!(game_name, "f14_l1");
            }
            other => panic!("expected Required, got {other:?}"),
        }
    }

    #[test]
    fn controller_vbs_alone_does_not_imply_a_rom() {
        // A real case: the VPX demo table loads controller.vbs for DOF and does
        // not use PinMAME. Nor is it fooled by commented-out cGameNames.
        let script = r#"
            'First, try to load the Controller.vbs (DOF), which helps
            'controlling additional hardware like lights and knockers
            On Error Resume Next
            ExecuteGlobal GetTextFile("controller.vbs")
            If Err Then MsgBox "You need the Controller.vbs file"
            'Const cGameName = ""
        "#;
        assert_eq!(detect(script), RomRequirement::NotNeeded);
    }

    #[test]
    fn cgamename_without_pinmame_is_a_dof_identifier() {
        // A real case: "Nudge Test and Calibration" (DJRobX). It declares
        // cGameName but never touches PinMAME: it is the id DOF uses.
        let script = r#"
            Const cGameName = "dof_test"
            ExecuteGlobal GetTextFile("controller.vbs")
        "#;
        assert_eq!(detect(script), RomRequirement::NotNeeded);
    }

    #[test]
    fn it_uses_vpinmame_but_with_no_readable_name() {
        let script = r#"
            Set c = CreateObject("VPinMAME.Controller")
            c.GameName = ResolveRomName()
        "#;
        assert_eq!(detect(script), RomRequirement::RequiredUnknown);
    }
}
