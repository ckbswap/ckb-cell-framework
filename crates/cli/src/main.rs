//! Workspace command entry point. The CLI invokes existing reproducible build
//! and VM test scripts and never signs or broadcasts a transaction.
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

const HELP: &str = "CKB Cell Framework\n\nUsage: cargo cell <command>\n\n  new <name>       Create a contract and VM tests from Counter\n  build [package]  Build a RISC-V contract (default: counter)\n  idl <package> [--out path]  Export a registered JSON interface\n  test             Format check, native tests, builds and VM tests\n  help             Show this help\n\nRun commands inside the framework workspace.\n";

#[derive(Debug, PartialEq, Eq)]
enum Task {
    Help,
    New(String),
    Build(Option<String>),
    Idl {
        package: String,
        output: Option<String>,
    },
    Test,
}

fn parse(mut args: Vec<String>) -> Result<Task, String> {
    if args.first().is_some_and(|arg| arg == "cell") {
        args.remove(0);
    }
    match args.as_slice() {
        [] => Ok(Task::Help),
        [command] if matches!(command.as_str(), "help" | "--help" | "-h") => Ok(Task::Help),
        [command, name] if command == "new" => Ok(Task::New(name.clone())),
        [command] if command == "build" => Ok(Task::Build(None)),
        [command, package] if command == "build" => Ok(Task::Build(Some(package.clone()))),
        [command, package] if command == "idl" && !package.starts_with('-') => Ok(Task::Idl {
            package: package.clone(),
            output: None,
        }),
        [command, package, flag, output]
            if command == "idl" && !package.starts_with('-') && flag == "--out" =>
        {
            Ok(Task::Idl {
                package: package.clone(),
                output: Some(output.clone()),
            })
        }
        [command] if command == "test" => Ok(Task::Test),
        _ => Err("unknown command or incorrect argument count; use `cargo cell help`".into()),
    }
}

fn workspace(start: &Path) -> Result<PathBuf, String> {
    start
        .ancestors()
        .find(|path| {
            fs::read_to_string(path.join("Cargo.toml"))
                .is_ok_and(|text| text.lines().any(|line| line.trim() == "[workspace]"))
                && path.join("scripts/test.sh").is_file()
        })
        .map(Path::to_path_buf)
        .ok_or_else(|| "no framework workspace found in current directory or ancestors".into())
}

fn run(task: Task) -> Result<(), String> {
    if task == Task::Help {
        print!("{HELP}");
        return Ok(());
    }
    let root = workspace(&env::current_dir().map_err(|e| e.to_string())?)?;
    let mut command = match task {
        Task::New(name) => {
            let mut command = Command::new("python3");
            command.arg("scripts/new-contract.py").arg(name);
            command
        }
        Task::Build(package) => {
            let mut command = Command::new("bash");
            command.arg("scripts/build.sh");
            if let Some(package) = package {
                command.arg(package);
            }
            command
        }
        Task::Test => {
            let mut command = Command::new("bash");
            command.arg("scripts/test.sh");
            command
        }
        Task::Idl { package, output } => {
            let mut command = Command::new("python3");
            command.arg("scripts/export-idl.py").arg(package);
            if let Some(output) = output {
                command.arg("--out").arg(output);
            }
            command
        }
        Task::Help => unreachable!(),
    };
    let status = command
        .current_dir(root)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("command failed: {status}"));
    }
    Ok(())
}

fn main() -> ExitCode {
    match parse(env::args().skip(1).collect()).and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_cargo_subcommand_and_alias_invocation_without_shell_interpolation() {
        let args = |values: &[&str]| values.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            parse(args(&["cell", "build", "framework-limit-order-script"])),
            Ok(Task::Build(Some("framework-limit-order-script".into())))
        );
        assert_eq!(
            parse(args(&["new", "my-vault"])),
            Ok(Task::New("my-vault".into()))
        );
        assert_eq!(parse(args(&["test"])), Ok(Task::Test));
        assert_eq!(parse(Vec::new()), Ok(Task::Help));
        assert!(parse(args(&["new"])).is_err());
        assert!(parse(args(&["new", "my-vault", "--unknown"])).is_err());
        assert!(parse(args(&["test", "extra"])).is_err());
        assert!(parse(args(&["publish"])).is_err());
        assert_eq!(
            parse(args(&["idl", "framework-vesting-script"])),
            Ok(Task::Idl {
                package: "framework-vesting-script".into(),
                output: None
            })
        );
        assert_eq!(
            parse(args(&[
                "idl",
                "framework-vesting-script",
                "--out",
                "path with spaces.json"
            ])),
            Ok(Task::Idl {
                package: "framework-vesting-script".into(),
                output: Some("path with spaces.json".into())
            })
        );
        assert!(parse(args(&["idl"])).is_err());
        assert!(parse(args(&["idl", "--out"])).is_err());
    }

    #[test]
    fn locates_workspace_from_nested_contract_directory() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        assert_eq!(
            workspace(&root.join("examples/counter/contract")).unwrap(),
            root
        );
    }
}
