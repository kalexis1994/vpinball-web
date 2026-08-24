//! The sound board, with no host around it, callable from anywhere.
//!
//! Built for one question: how much slower is this in a browser than it is
//! here? The answer decides whether the work is to make the emulator faster or
//! to look somewhere else entirely, and guessing at it is how a week goes into
//! optimising something that was never the problem.
//!
//! Deliberately not `wasm-bindgen`: plain C exports, so the same library runs
//! natively and under a bare `WebAssembly.instantiate` with no glue between
//! the measurement and the thing being measured.

use std::sync::Mutex;

use vpw_arm7::Cpu;
use vpw_at91::Sound;

struct Board {
    cpu: Cpu,
    sound: Box<Sound>,
    cycles: u64,
}

static BOARD: Mutex<Option<Board>> = Mutex::new(None);
static BUFFERS: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

/// Space for the caller to put an image in. Returns where to put it.
///
/// # Safety
/// The pointer is valid until [`reset`].
#[unsafe(no_mangle)]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut held = BUFFERS.lock().unwrap();
    held.push(vec![0u8; len]);
    held.last_mut().unwrap().as_mut_ptr()
}

/// Hands one of the board's six images over. `which` is 0 for the BIOS, 1 for
/// U7, and 2 to 5 for the four sample parts.
#[unsafe(no_mangle)]
pub extern "C" fn load(which: u32, index: usize) {
    let held = BUFFERS.lock().unwrap();
    let Some(image) = held.get(index) else { return };
    let mut board = BOARD.lock().unwrap();
    let board = board.get_or_insert_with(|| Board {
        cpu: {
            let mut c = Cpu::new();
            c.reset();
            c
        },
        sound: Box::new(Sound::new()),
        cycles: 0,
    });
    match which {
        0 => board.sound.load_bios(image),
        1 => board.sound.load_u7(image),
        n @ 2..=5 => board.sound.load_samples((n - 2) as usize, image),
        _ => {}
    }
}

/// Runs the board for `cycles` of its own forty-megahertz clock and says how
/// many samples came out.
#[unsafe(no_mangle)]
pub extern "C" fn run(cycles: u32) -> u32 {
    let mut held = BOARD.lock().unwrap();
    let Some(board) = held.as_mut() else { return 0 };
    let until = board.cycles + u64::from(cycles);
    let mut made = 0u32;
    while board.cycles < until {
        let n = board.cpu.step(&mut *board.sound);
        board.cycles += u64::from(n);
        board.cpu.irq = board.sound.tick(n);
        board.cpu.fiq = board.sound.fiq();
        if board.sound.queued() > 4096 {
            made += board.sound.take_audio().len() as u32;
        }
    }
    made + board.sound.take_audio().len() as u32
}
