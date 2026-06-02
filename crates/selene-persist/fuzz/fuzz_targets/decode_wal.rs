#![no_main]

use libfuzzer_sys::fuzz_target;
use selene_persist::WalReader;

// Drives the whole streaming WAL decode on attacker bytes: SLDB file-header
// decode, per-entry header decode (fixed + replicated tail + principal),
// payload-length-vs-remaining bound, and `body()` = xxh3 checksum + bounded
// zstd decompress + postcard `Vec<Change>` decode. Only a panic/OOM/hang fails;
// any malformed input yields a typed error and the stream self-stops.
fuzz_target!(|bytes: &[u8]| {
    let Ok(stream) = WalReader::from_bytes(bytes) else {
        return;
    };
    for item in stream {
        match item {
            Ok(view) => {
                let _ = view.body();
            }
            Err(_) => break,
        }
    }
});
