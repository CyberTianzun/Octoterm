//! A child of ConPTY for ws_keyboard.rs. Read the actual Windows input records,
//! rather than Console.ReadKey (which filters some Alt combinations).
use std::ffi::c_void;
use std::io::Write;

#[repr(C)]
#[derive(Default)]
struct KeyRecord {
    down: i32,
    repeat: u16,
    vk: u16,
    scan: u16,
    character: u16,
    state: u32,
}

#[repr(C)]
#[derive(Default)]
struct InputRecord {
    kind: u16,
    padding: u16,
    key: KeyRecord,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetStdHandle(which: u32) -> *mut c_void;
    fn GetConsoleMode(handle: *mut c_void, mode: *mut u32) -> i32;
    fn SetConsoleMode(handle: *mut c_void, mode: u32) -> i32;
    fn ReadConsoleInputW(
        handle: *mut c_void,
        record: *mut InputRecord,
        size: u32,
        read: *mut u32,
    ) -> i32;
}

fn main() {
    unsafe {
        let input = GetStdHandle(-10i32 as u32);
        let mut mode = 0;
        assert_ne!(GetConsoleMode(input, &mut mode), 0);
        // Same raw-mode flags as crossterm; do not replace the inherited mode.
        assert_ne!(SetConsoleMode(input, mode & !0x7), 0);
        println!("KEYBOARD_READY");
        std::io::stdout().flush().unwrap();
        loop {
            let mut record = InputRecord::default();
            let mut read = 0;
            assert_ne!(ReadConsoleInputW(input, &mut record, 1, &mut read), 0);
            if record.kind != 1 {
                continue;
            }
            let k = record.key;
            println!(
                "KEY down={} vk={} scan={} char={} state={} repeat={}",
                k.down, k.vk, k.scan, k.character, k.state, k.repeat
            );
            std::io::stdout().flush().unwrap();
        }
    }
}
