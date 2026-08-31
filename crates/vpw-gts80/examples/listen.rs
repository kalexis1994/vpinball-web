//! Boots a System 80 set, plays a coin and a game into it, and writes what
//! the sound card made to a WAV.
//!
//! ```text
//! cargo run -p vpw-gts80 --example listen -- <rom directory> [out.wav] [trough...]
//! ```
//!
//! The ROMs are copyrighted firmware and are not in the repository; point this
//! at a directory holding an unpacked set.
//!
//! The coin and start buttons are the same on every one of these machines —
//! `sys80.vbs` names them, and every System 80 table loads that file. What is
//! *not* the same is where the balls sit: a machine will not start a game
//! without one where it can find it, and which switch that is depends on the
//! table. Ice Fever's is 67 and it is the default; Genesis wants 55 and 74.
//! They come off the table's own script, from its `bsTrough.InitSw` line.

use std::path::PathBuf;

use vpw_gts80::Board;
use vpw_gts80::games::Sound;

/// The rate the player's mixer runs at.
const RATE: u32 = 48_000;

/// Where Ice Fever keeps its ball, for when nobody says otherwise.
const DEFAULT_TROUGH: [i32; 1] = [67];
/// The third coin chute, which gives a credit for one coin on a board whose
/// memory is still blank. `sys80.vbs:26`.
const COIN: i32 = 37;
/// `sys80.vbs:27`.
const START: i32 = 47;

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("a rom directory"));
    let wav_path = args.next().filter(|p| p != "-");
    let trough: Vec<i32> = args.filter_map(|a| a.parse().ok()).collect();
    let trough: &[i32] = if trough.is_empty() {
        &DEFAULT_TROUGH
    } else {
        &trough
    };

    let mut images = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("the directory will not open") {
        let path = entry.expect("a directory entry").path();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        images.push((name, std::fs::read(&path).expect("a file will not open")));
    }

    // The set name is only used to tell two of the System 80B sound cards
    // apart, so the directory's own name will do.
    let set = dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let roms = vpw_gts80::games::detect(&set, &images).expect("not a System 80 set");
    println!(
        "sound  {}",
        match roms.sound {
            Some(Sound::Piggyback(rom)) => format!("piggyback card, {} bytes", rom.len()),
            Some(Sound::Card { rom, system }) => format!(
                "sound card, {} bytes and the 6530's {}",
                rom.len(),
                system.len()
            ),
            Some(Sound::Speech { first, second }) => format!(
                "sound & speech board, {} + {} bytes and a Votrax",
                first.len(),
                second.len()
            ),
            Some(Sound::Card80B { generation, y, d }) => format!(
                "System 80B card, {generation:?}: {} bytes of music and {} of converter",
                y.0.len() + y.1.map_or(0, <[u8]>::len),
                d.len()
            ),
            None => "no card this port knows; the machine will be quiet".to_string(),
        }
    );

    let mut board = Board::from_roms(&roms, RATE);
    if let Some(card) = &board.sound {
        println!("       clocked at {} Hz", card.clock());
    }

    let mut wav = Vec::new();
    let take = |board: &mut Board, wav: &mut Vec<f32>, what: &str| {
        let audio = board.take_audio_at(RATE);
        let peak = audio.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let power = audio.iter().map(|s| s * s).sum::<f32>() / audio.len().max(1) as f32;
        println!(
            "{what:<9} {:6} samples, peak {peak:.3}, rms {:.4}",
            audio.len(),
            power.sqrt()
        );
        wav.extend_from_slice(&audio);
    };

    // How long to let it settle before putting a coin in. Three seconds is
    // enough for Ice Fever; the early two-PROM sets take far longer to finish
    // powering up — Circus has nothing on its displays at two seconds and is
    // fully awake by fifteen — and a coin offered to a machine that has not
    // finished booting is a coin it never sees.
    let settle: f64 = std::env::var("GTS80_SETTLE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3.0);
    board.run_seconds(settle);
    take(&mut board, &mut wav, "attract");

    for &switch in trough {
        board.mem.io.set_switch(switch, true);
    }
    board.run_seconds(0.5);
    for _ in 0..3 {
        board.mem.io.set_switch(COIN, true);
        board.run_seconds(0.2);
        board.mem.io.set_switch(COIN, false);
        board.run_seconds(0.4);
    }
    take(&mut board, &mut wav, "coins");

    board.mem.io.set_switch(START, true);
    board.run_seconds(0.3);
    board.mem.io.set_switch(START, false);
    // How long to listen after the game begins, and every command heard while
    // listening: a machine that says nothing and a machine we never asked are
    // the same silence otherwise.
    let play: f64 = std::env::var("GTS80_PLAY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3.0);
    let mut heard: Vec<u8> = Vec::new();
    let step: f64 = 0.02;
    let mut left = play;
    while left > 0.0 {
        board.run_seconds(step.min(left));
        left -= step;
        let c = board.mem.io.sound_command;
        if c != 0 && !heard.contains(&c) {
            heard.push(c);
        }
    }
    // A machine with a game on and nothing happening on the playfield has
    // nothing to say. `GTS80_POKE=1` closes every switch in turn — a bumper, a
    // target, a rollover — and says which ones made a noise.
    if std::env::var("GTS80_POKE").is_ok() {
        let mut spoke: Vec<(i32, u8)> = Vec::new();
        for sw in 1..=77 {
            board.mem.io.set_switch(sw, true);
            board.run_seconds(0.12);
            board.mem.io.set_switch(sw, false);
            board.run_seconds(0.12);
            let c = board.mem.io.sound_command;
            if c != 0 {
                spoke.push((sw, c));
            }
        }
        println!("       switches that made a sound: {spoke:02x?}");
    }
    take(&mut board, &mut wav, "start");
    println!("       commands heard: {heard:02x?}");
    println!(
        "       every bit ever put on that port: {:#04x}, direction now {:#04x}",
        board.mem.io.sound_port_seen,
        board.mem.riot[2].ddr_a()
    );

    // The one line that says whether any of this worked: the game-on relay,
    // which a table reads as solenoid 10 and without which no flipper moves.
    println!(
        "       game on: {}",
        if board.mem.io.solenoid_on(10) {
            "yes"
        } else {
            "no"
        }
    );

    let (_, made) = board.sound_stats();
    println!(
        "       {made} samples in all, last command {:#04x}",
        board.mem.io.sound_command
    );

    if let Some(path) = wav_path {
        std::fs::write(&path, wave(&wav, RATE)).expect("the wav will not write");
        println!("wrote  {path}");
    }
}

/// Mono sixteen-bit PCM, which is the least a player will open.
fn wave(samples: &[f32], rate: u32) -> Vec<u8> {
    let bytes = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + bytes).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&bytes.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}
