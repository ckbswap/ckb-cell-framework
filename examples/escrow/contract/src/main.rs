//! CKB-VM entry point. Transaction validation is in lib.rs; business rules are in state.rs.
#![no_std]
#![no_main]

// Generate VM startup code and provide the default allocator for no_std.
ckb_std::entry!(entry);
ckb_std::default_alloc!();

fn entry() -> i8 {
    // Return zero only after all checks pass; otherwise return the framework error code.
    framework_escrow_script::verify().map_or_else(|error| error as i8, |_| 0)
}
