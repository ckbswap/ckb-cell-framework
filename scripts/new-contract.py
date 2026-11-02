#!/usr/bin/env python3
"""Create one contract library/entry crate and VM tests from the Counter example."""
import argparse
import json
from pathlib import Path
import re
import shutil
import subprocess


def append_members(manifest: str, key: str, additions: list) -> str:
    match = re.search(rf"(?m)^{re.escape(key)}\s*=\s*\[([^]]*)\]", manifest)
    if not match:
        raise ValueError(f"Unsupported workspace manifest: missing {key}")
    previous = match.group(1).rstrip().rstrip(",")
    entries = ", ".join(f'"{entry}"' for entry in additions)
    return manifest[:match.start(1)] + previous + ", " + entries + manifest[match.end(1):]


def create_contract(root: Path, name: str) -> Path:
    example = "counter"
    prefix = "framework-counter"
    if not re.fullmatch(r"[a-z][a-z0-9]*(?:-[a-z0-9]+)*", name) or len(name) > 40:
        raise ValueError("Use a lowercase kebab-case name of at most 40 characters")
    destination = root / "contracts" / name
    if destination.exists():
        raise ValueError("Destination already exists; no files were changed")
    manifest_path = root / "Cargo.toml"
    manifest = manifest_path.read_text()
    if "[workspace.dependencies]" not in manifest:
        raise ValueError("Unsupported workspace manifest")
    if re.search(rf"(?m)^{re.escape(name)}-script\s*=", manifest):
        raise ValueError("Workspace dependency name already exists")
    try:
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--offline", "--no-deps", "--format-version", "1"],
            cwd=root, text=True, stderr=subprocess.PIPE,
        ))
    except subprocess.CalledProcessError as error:
        raise ValueError(f"Cannot inspect workspace packages: {error.stderr.strip()}") from error
    except OSError as error:
        raise ValueError(f"Cannot inspect workspace packages: {error}") from error
    members = set(metadata["workspace_members"])
    packages = {p["name"] for p in metadata["packages"] if p["id"] in members}
    for package in [f"{name}-script", f"{name}-vm-tests"]:
        if package in packages:
            raise ValueError(f"Workspace package name already exists: {package}")
    manifest = append_members(manifest, "members", [
        f"contracts/{name}/{part}" for part in ["contract", "tests"]
    ])
    manifest = append_members(manifest, "default-members", [f"contracts/{name}/contract"])
    manifest = manifest.replace(
        "[workspace.dependencies]",
        f'[workspace.dependencies]\n{name}-script = {{ path = "contracts/{name}/contract" }}',
        1,
    )
    template = root / "examples" / example
    if not all((template / "contract" / path).is_file() for path in ["Cargo.toml", "src/lib.rs", "src/main.rs", "src/state.rs", "src/cells.rs"]):
        raise ValueError("Counter template is missing")
    vm_template = root / "tests/src/tests.rs"
    if not vm_template.is_file():
        raise ValueError("Counter VM test template is missing")
    destination.mkdir(parents=True)
    try:
        shutil.copytree(template / "contract", destination / "contract")
        tests = destination / "tests"
        (tests / "src").mkdir(parents=True)
        (tests / "Cargo.toml").write_text(f'''[package]
name = "{name}-vm-tests"
version.workspace = true
edition.workspace = true
license.workspace = true
publish.workspace = true

[features]
vm-tests = []

[dev-dependencies]
ckb-testtool.workspace = true
ckb-contract-core.workspace = true
ckb-contract-tx.workspace = true
ckb-contract-testkit.workspace = true
{name}-script.workspace = true
''')
        (tests / "src/lib.rs").write_text('#[cfg(all(test, feature = "vm-tests"))]\nmod tests;\n')
        vm_source = vm_template.read_text()
        (tests / "src/tests.rs").write_text(vm_source)
        for path in destination.rglob("*"):
            if path.is_file() and path.suffix in {".toml", ".rs"}:
                text = path.read_text().replace(prefix, name)
                text = text.replace(prefix.replace("-", "_"), name.replace("-", "_"))
                if path == destination / "contract/Cargo.toml":
                    text = text.replace('vm-tests = "framework-vm-tests"', f'vm-tests = "{name}-vm-tests"')
                path.write_text(text)
        manifest_path.write_text(manifest)
    except Exception:
        shutil.rmtree(destination)
        raise
    return destination


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("name", help="e.g. my-vault")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    try:
        destination = create_contract(root, args.name)
    except ValueError as error:
        parser.exit(2, f"{error}\n")
    print(f"Created {destination.relative_to(root)}")
    print(f"cargo check -p {args.name}-script   # checks the library and updates Cargo.lock")
    print(f"bash scripts/build.sh {args.name}-script")
    print("bash scripts/test.sh   # includes the new contract library, entry and VM tests")


if __name__ == "__main__":
    main()
