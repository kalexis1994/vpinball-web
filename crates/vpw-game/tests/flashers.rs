//! A flasher as the script sees it, on a real table.
//!
//! F-14 has three flashers — `ref1`, `ref2`, `ref3`, the reflections on the
//! side plastics — and nothing about them is special; what is tested is that
//! every member `flasher.cpp:553-830` answers to is there, comes back as it
//! was written, and reaches the block the renderer reads.

use std::rc::Rc;

use vpw_game::items::Kind;
use vpw_game::{Game, LibraryDir, Resources, ScriptLibrary};
use vpw_table::flasher::Filter;
use vpw_vbscript::Object;
use vpw_vbscript::value::Value;

/// The table, or nothing if it is not here. See `targets.rs` for why this is
/// a skip and not a failure.
fn table_bytes() -> Option<Vec<u8>> {
    const PATH: &str = "../../web/debug-assets/f14.vpx";
    match std::fs::read(PATH) {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            eprintln!("skipped: {PATH} is not there");
            None
        }
    }
}

/// The table loaded and not started: the parts exist before the script runs.
fn loaded() -> Option<Game> {
    let bytes = table_bytes()?;
    let vpx = vpin::vpx::from_bytes(&bytes).expect("a readable .vpx");
    let mut scene = vpw_table::geometry::extract(&vpx);
    let libraries: Rc<dyn ScriptLibrary> = Rc::new(LibraryDir(
        std::path::Path::new("../../../vpinball/scripts").into(),
    ));
    Some(Game::load(&vpx, &mut scene, Resources::new(libraries)).expect("the table should load"))
}

fn set(item: &dyn Object, name: &str, value: Value) {
    item.set(name, &[], value, false)
        .unwrap_or_else(|e| panic!("{name}: {e}"));
}

fn get(item: &dyn Object, name: &str) -> Value {
    item.get(name, &[])
        .unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// A member read back as a number, the way a script comparing it would.
fn num(item: &dyn Object, name: &str) -> f64 {
    get(item, name).to_number().unwrap()
}

fn text(item: &dyn Object, name: &str) -> String {
    get(item, name).to_str().unwrap().to_string()
}

#[test]
fn a_flasher_starts_where_the_file_leaves_it() {
    let Some(game) = loaded() else { return };
    let item = game.items().get("ref1").expect("F-14 has ref1");
    assert_eq!(item.kind, Kind::Flasher);
    let s = item.flasher_state();
    // The values the inventory of the file shows: fifty-five up, white,
    // opacity twenty, the same picture on both sides, additive filter at
    // twenty per cent, painted rather than additive.
    assert_eq!(s.height, 55.0);
    assert_eq!(s.color, [1.0, 1.0, 1.0]);
    assert_eq!(s.alpha, 20.0);
    assert_eq!(s.intensity_scale, 1.0);
    assert!(s.image_a.eq_ignore_ascii_case("reflect_01"));
    assert_eq!(s.image_a, s.image_b);
    assert_eq!(s.filter, Filter::Additive);
    assert_eq!(s.filter_amount, 20.0);
    assert!(!s.add_blend);
    // The file saves it shown and the script hides it on the way in — F-14's
    // `Table1_Init` runs `ref1.visible=0` and shows the reflections again
    // with the general illumination — and it is the script's word that has
    // to reach the renderer, or the reflections sit on a dark table.
    assert!(!s.visible && !s.is_drawn());
    set(&*item, "Visible", Value::Bool(true));
    assert!(item.flasher_state().is_drawn());
    // The centre the script reads is the centre the renderer turns about.
    let scene_flasher = {
        let bytes = table_bytes().unwrap();
        let vpx = vpin::vpx::from_bytes(&bytes).unwrap();
        let scene = vpw_table::geometry::extract(&vpx);
        scene
            .flashers
            .iter()
            .find(|f| f.name == "ref1")
            .expect("built")
            .clone()
    };
    assert_eq!((s.x, s.y), (scene_flasher.center.x, scene_flasher.center.y));
    assert_eq!(num(&*item, "X"), f64::from(s.x));
}

#[test]
fn every_member_a_script_writes_comes_back_and_reaches_the_renderer() {
    let Some(game) = loaded() else { return };
    let item = game.items().get("ref2").expect("F-14 has ref2");
    let obj: &dyn Object = &*item;

    // What `core.vbs` does off a solenoid (`core.vbs:2534`) and what JP's
    // fading flashers do every frame.
    set(obj, "Visible", Value::Bool(false));
    set(obj, "IntensityScale", Value::Double(0.25));
    set(obj, "Opacity", Value::Long(60));
    // An `OLE_COLOR`: `0x00BBGGRR`, so this is pure blue.
    set(obj, "Color", Value::Long(0xFF0000));
    set(obj, "ImageA", Value::Str(Rc::from("flash_a")));
    set(obj, "ImageB", Value::Str(Rc::from("")));
    set(obj, "X", Value::Double(300.0));
    set(obj, "Y", Value::Double(900.0));
    set(obj, "Height", Value::Double(120.0));
    set(obj, "RotX", Value::Double(-45.0));
    set(obj, "RotY", Value::Double(5.0));
    set(obj, "RotZ", Value::Double(90.0));
    set(obj, "AddBlend", Value::Bool(true));
    set(obj, "ModulateVsAdd", Value::Double(0.5));
    set(obj, "Filter", Value::Str(Rc::from("multiply")));
    set(obj, "Amount", Value::Long(400));

    assert!(!get(obj, "Visible").to_bool().unwrap());
    assert_eq!(num(obj, "Opacity"), 60.0);
    assert_eq!(num(obj, "Color"), f64::from(0xFF0000));
    assert_eq!(text(obj, "ImageA"), "flash_a");
    assert_eq!(num(obj, "Height"), 120.0);
    assert_eq!(num(obj, "RotZ"), 90.0);
    assert!(get(obj, "AddBlend").to_bool().unwrap());
    // `get_Filter` answers the capitalised name (`flasher.cpp:692-708`).
    assert_eq!(text(obj, "Filter"), "Multiply");
    assert_eq!(num(obj, "Amount"), 400.0);

    let s = item.flasher_state();
    assert!(!s.visible);
    assert_eq!(s.intensity_scale, 0.25);
    assert_eq!(s.alpha, 60.0);
    assert_eq!(s.color, [0.0, 0.0, 1.0]);
    assert_eq!(s.image_a, "flash_a");
    assert_eq!(s.image_b, "");
    assert_eq!((s.x, s.y, s.height), (300.0, 900.0, 120.0));
    assert_eq!(s.rot, [-45.0, 5.0, 90.0]);
    assert!(s.add_blend);
    assert_eq!(s.modulate_vs_add, 0.5);
    assert_eq!(s.filter, Filter::Multiply);
    assert_eq!(s.filter_amount, 400.0);
    // The shader gets alpha times scale out of a hundred (`flasher.cpp:1241`).
    assert!((s.shader_color()[3] - 0.15).abs() < 1e-6);
}

#[test]
fn opacity_is_a_long_and_never_negative() {
    // `SetAlpha` clamps at zero (`flasher.h:154-157`) and the interface
    // declares a `LONG`, so a fraction is gone before the flasher sees it.
    let Some(game) = loaded() else { return };
    let item = game.items().get("ref3").expect("F-14 has ref3");
    let obj: &dyn Object = &*item;
    set(obj, "Opacity", Value::Double(-30.0));
    assert_eq!(num(obj, "Opacity"), 0.0);
    assert!(!item.flasher_state().is_drawn(), "at zero it is not drawn");
    set(obj, "Opacity", Value::Double(12.7));
    assert_eq!(num(obj, "Opacity"), 13.0);
}

#[test]
fn an_unknown_filter_name_changes_nothing() {
    // `put_Filter` matches five names and otherwise leaves the value alone
    // (`flasher.cpp:710-726`).
    let Some(game) = loaded() else { return };
    let item = game.items().get("ref1").expect("F-14 has ref1");
    let obj: &dyn Object = &*item;
    set(obj, "Filter", Value::Str(Rc::from("Screen")));
    set(obj, "Filter", Value::Str(Rc::from("plaid")));
    assert_eq!(text(obj, "Filter"), "Screen");
}
