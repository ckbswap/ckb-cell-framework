#!/usr/bin/env python3
"""Discover registered workspace contracts, rebuild them, then run their VM tests."""
import json
from pathlib import Path
import subprocess


def contract_plan(metadata: dict) -> tuple:
    members = set(metadata["workspace_members"])
    packages = {p["name"]: p for p in metadata["packages"] if p["id"] in members}
    contracts = []
    tests = set()
    for package in packages.values():
        config = (package.get("metadata") or {}).get("ckb-contract")
        if config is None:
            continue
        test_name = config.get("vm-tests")
        test_package = packages.get(test_name)
        if test_package is None or "vm-tests" not in test_package["features"]:
            raise ValueError(f'{package["name"]}: vm-tests must name a workspace package with the vm-tests feature')
        if not any("bin" in t["kind"] and t["name"] == package["name"] for t in package["targets"]):
            raise ValueError(f'{package["name"]}: contract binary must have the same name as its package')
        contracts.append(package["name"])
        tests.add(test_name)
    if not contracts:
        raise ValueError("No contracts registered with package.metadata.ckb-contract")
    return sorted(contracts), sorted(tests)


def main():
    root = Path(__file__).resolve().parent.parent
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], cwd=root,
    ))
    try:
        contracts, tests = contract_plan(metadata)
    except ValueError as error:
        raise SystemExit(str(error))
    for name in contracts:
        subprocess.run(["bash", "scripts/build.sh", name], cwd=root, check=True)
    for package in metadata["packages"]:
        if package["name"] in contracts and (package.get("metadata") or {}).get("ckb-contract", {}).get("idl"):
            subprocess.run(["python3", "scripts/export-idl.py", package["name"]], cwd=root, check=True)
    subprocess.run(["python3", "scripts/fetch-udt.py"], cwd=root, check=True)
    command = ["cargo", "test", "--locked"]
    for name in tests:
        command.extend(["-p", name])
    subprocess.run(command + ["--features", "vm-tests"], cwd=root, check=True)


if __name__ == "__main__":
    main()
