#![no_main]

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = tysel_task_rpc::decode_message(data);
    let _ = tysel_task_rpc::read_message_opt(&mut Cursor::new(data));
});
