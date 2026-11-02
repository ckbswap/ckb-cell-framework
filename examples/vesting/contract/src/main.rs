//! VM startup only. Contract logic is in lib.rs.
#![no_std]
#![no_main]

ckb_std::entry!(entry);
ckb_std::default_alloc!();

fn entry() -> i8 {
    framework_vesting_script::verify().map_or_else(|error| error as i8, |_| 0)
}
