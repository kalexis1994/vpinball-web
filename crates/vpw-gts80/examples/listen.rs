//! Boots a System 80 set, plays a coin and a game into it, and writes what
//! the sound card made to a WAV.
//!
//! ```text
//! cargo run -p vpw-gts80 --example listen -- <rom directory> [out.wav]
//! ```
//!
//! The ROMs are copyrighted firmware and are not in the repository; point this
//! at a directory holding an unpacked set. The switch numbers below are Ice
//! Fever's, because a machine will not start a game without a ball where it
//! can find one, and where that is depends on the table.

use std::path::PathBuf;

use vpw_gts80::Board;
use vpw_gts80::games::Sound;

/// The rate the player's mixer runs at.
const RATE: u32 = 48_000;

/// `bsTrough.InitSw 0,67,…` in Ice Fever's own script.
const OUTHOLE: i32 = 67;
/// The third coin chute, which gives a credit for one coin on a board whose
/// memory is still blank.
const COIN: i32 = 37;
const START: i32 = 47;

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("a rom directory"));
    let wav_path = args.next();

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

    let roms = vpw_gts80::games::detect(&images).expect("not a System 80 set");
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
            None => "no card this port knows; the machine will be quiet".to_string(),
        }
    );

    let mut board = Board::new(roms.game.to_vec(), roms.u2.to_vec(), roms.u3.to_vec());
    if let Some(sound) = roms.sound {
        board.load_sound(sound, RATE);
    }
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

    board.run_seconds(3.0);
    take(&mut board, &mut wav, "attract");

    board.mem.io.set_switch(OUTHOLE, true);
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
    board.run_seconds(3.0);
    take(&mut board, &mut wav, "start");

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
