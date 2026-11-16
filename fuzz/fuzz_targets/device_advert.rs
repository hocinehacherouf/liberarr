#![no_main]

use liberarr_core::device::protocol::parse_advert;
use libfuzzer_sys::fuzz_target;

// Manufacturer data from any nearby BLE radio.
fuzz_target!(|data: &[u8]| {
    let _ = parse_advert(data);
});
