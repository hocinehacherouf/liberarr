#![no_main]

use liberarr_core::device::protocol::{
    frame_id, parse_data_packet, parse_download_status, parse_handshake_reply, parse_list_page,
    parse_secret_part, reply_cmd,
};
use libfuzzer_sys::fuzz_target;

// Packets received from the device over BLE.
fuzz_target!(|data: &[u8]| {
    let _ = reply_cmd(data);
    let _ = frame_id(data);
    let _ = parse_handshake_reply(data);
    let _ = parse_list_page(data);
    let _ = parse_download_status(data);
    let _ = parse_data_packet(data);
    let _ = parse_secret_part(data);
});
