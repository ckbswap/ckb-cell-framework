#!/usr/bin/env python3
"""Run a registered host exporter; generated JSON contains no network deployment."""
import argparse
import json
from pathlib import Path
import subprocess


def export_plan(metadata: dict, name: str) -> str:
    members = set(metadata["workspace_members"])
    package = next((p for p in metadata["packages"] if p["id"] in members and p["name"] == name), None)
    if package is None:
        raise ValueError(f"Unknown workspace package: {name}")
    config = (package.get("metadata") or {}).get("ckb-contract") or {}
    binary = config.get("idl")
    if not isinstance(binary, str) or not binary:
        raise ValueError(f"{name}: no IDL exporter registered in package.metadata.ckb-contract.idl")
    if "idl" not in package["features"] or not any(
        target["name"] == binary and "bin" in target["kind"] and "idl" in target.get("required-features", [])
        for target in package["targets"]
    ):
        raise ValueError(f"{name}: exporter must be a binary requiring the idl feature")
    return binary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package")
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], cwd=root,
    ))
    try:
        binary = export_plan(metadata, args.package)
    except ValueError as error:
        parser.exit(2, f"{error}\n")
    output = args.out if args.out is not None else Path("build/idl") / f"{args.package}.json"
    subprocess.run([
        "cargo", "run", "--quiet", "--locked", "-p", args.package,
        "--features", "idl", "--bin", binary, "--", str(output),
    ], cwd=root, check=True)


if __name__ == "__main__":
    main()
