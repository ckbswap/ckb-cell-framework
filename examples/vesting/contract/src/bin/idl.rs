use std::{env, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let path = args.next().map(PathBuf::from);
    if path.is_none() || args.next().is_some() {
        eprintln!("usage: framework-vesting-idl <output.json>");
        return ExitCode::FAILURE;
    }
    let path = path.unwrap();
    match ckb_contract_idl::write(&framework_vesting_script::interface(), &path) {
        Ok(()) => {
            println!("Exported: {}", path.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
