#![no_main]

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = tysel_ipc::read_frame(&mut Cursor::new(data));
    let _ = tysel_ipc::read_message(&mut Cursor::new(data));
    let _ = serde_json::from_slice::<tysel_ipc::Message>(data);
});
