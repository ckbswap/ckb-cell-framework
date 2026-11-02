#!/usr/bin/env python3
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location("scaffold", ROOT / "scripts/new-contract.py")
SCAFFOLD = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SCAFFOLD)
PLAN_SPEC = importlib.util.spec_from_file_location("contracts", ROOT / "scripts/test-contracts.py")
CONTRACTS = importlib.util.module_from_spec(PLAN_SPEC)
PLAN_SPEC.loader.exec_module(CONTRACTS)
IDL_SPEC = importlib.util.spec_from_file_location("export_idl", ROOT / "scripts/export-idl.py")
IDL = importlib.util.module_from_spec(IDL_SPEC)
IDL_SPEC.loader.exec_module(IDL)


class ExportTests(unittest.TestCase):
    def test_exporter_registration_is_explicit_and_workspace_scoped(self):
        package = {"id": "member", "name": "demo", "features": {"idl": []},
                   "metadata": {"ckb-contract": {"idl": "demo-idl"}},
                   "targets": [{"name": "demo-idl", "kind": ["bin"], "required-features": ["idl"]}]}
        metadata = {"workspace_members": ["member"], "packages": [package]}
        self.assertEqual(IDL.export_plan(metadata, "demo"), "demo-idl")
        for missing in ["missing", "../demo", "demo;echo"]:
            with self.assertRaises(ValueError):
                IDL.export_plan(metadata, missing)
        metadata["workspace_members"] = []
        with self.assertRaises(ValueError):
            IDL.export_plan(metadata, "demo")

    def test_exporter_feature_and_target_must_be_registered(self):
        package = {"id": "member", "name": "demo", "features": {}, "metadata": {}, "targets": []}
        metadata = {"workspace_members": ["member"], "packages": [package]}
        with self.assertRaises(ValueError):
            IDL.export_plan(metadata, "demo")
        package["metadata"] = {"ckb-contract": {"idl": "missing"}}
        package["features"] = {"idl": []}
        with self.assertRaises(ValueError):
            IDL.export_plan(metadata, "demo")
        package["targets"] = [{"name": "missing", "kind": ["bin"]}]
        with self.assertRaises(ValueError):
            IDL.export_plan(metadata, "demo")


class ScaffoldTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        shutil.copy(ROOT / "Cargo.toml", self.root / "Cargo.toml")
        shutil.copytree(
            ROOT / "examples", self.root / "examples",
            ignore=shutil.ignore_patterns("node_modules", "dist"),
        )
        shutil.copytree(ROOT / "crates", self.root / "crates")
        shutil.copytree(ROOT / "tests", self.root / "tests")
        if (ROOT / "contracts").is_dir():
            shutil.copytree(ROOT / "contracts", self.root / "contracts")
        self.name = "scaffold-check"
        suffix = 0
        while (self.root / "contracts" / self.name).exists():
            suffix += 1
            self.name = f"scaffold-check-{suffix}"

    def test_generated_crates_and_workspace_are_consistent(self):
        target = SCAFFOLD.create_contract(self.root, self.name)
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--offline", "--no-deps", "--format-version", "1"],
            cwd=self.root,
        ))
        packages = {package["name"]: package for package in metadata["packages"]}
        self.assertIn(f"{self.name}-script", packages)
        self.assertNotIn(f"{self.name}-protocol", packages)
        self.assertFalse((target / "protocol").exists())
        self.assertIn(f"{self.name}-vm-tests", packages)
        defaults = set(metadata["workspace_default_members"])
        self.assertIn(packages[f"{self.name}-script"]["id"], defaults)
        self.assertNotIn(packages[f"{self.name}-vm-tests"]["id"], defaults)
        name = f"{self.name}-script"
        self.assertEqual(Path(packages[name]["manifest_path"]).resolve(), (target / "contract/Cargo.toml").resolve())
        self.assertTrue((target / "contract/src/state.rs").is_file())
        self.assertTrue((target / "contract/src/lib.rs").is_file())
        self.assertTrue((target / "contract/src/cells.rs").is_file())
        self.assertFalse((target / "contract/src/context.rs").exists())
        self.assertFalse((target / "contract/src/config.rs").exists())
        self.assertFalse((target / "contract/src/entry.rs").exists())
        targets = packages[name]["targets"]
        library = next(t for t in targets if "lib" in t["kind"])
        binary = next(t for t in targets if t["name"] == name and "bin" in t["kind"])
        self.assertEqual(Path(library["src_path"]).resolve(), (target / "contract/src/lib.rs").resolve())
        self.assertEqual(Path(binary["src_path"]).resolve(), (target / "contract/src/main.rs").resolve())
        self.assertEqual(binary.get("required-features", []), ["vm"])
        source = "\n".join(path.read_text() for path in (target / "contract/src").glob("*.rs"))
        self.assertNotIn("framework_counter_script", source)
        vm_source = (target / "tests/src/tests.rs").read_text()
        self.assertIn(f'load_binary(&root, "{self.name}-script")', vm_source)
        self.assertIn(f'use {self.name.replace("-", "_")}_script::{{Action, State, STATE_CAPACITY}};', vm_source)
        self.assertNotIn("RoleArgs", source)
        self.assertNotIn("StateCell", source)
        self.assertNotIn("LAYOUT", source)
        self.assertIn("Context<Create>", source)
        self.assertIn("#[derive(Cells)]", source)
        self.assertIn('workspace_root(env!("CARGO_MANIFEST_DIR"))', vm_source)
        dependencies = {dep["name"] for dep in packages[f"{self.name}-vm-tests"]["dependencies"]}
        self.assertTrue({"ckb-contract-tx", "ckb-contract-testkit", f"{self.name}-script"}.issubset(dependencies))
        contracts, tests = CONTRACTS.contract_plan(metadata)
        self.assertIn(f"{self.name}-script", contracts)
        self.assertIn(f"{self.name}-vm-tests", tests)
        self.assertEqual(tests.count("framework-vm-tests"), 1)
        # A typo must stop the pipeline before any build, rather than omit VM tests.
        packages[f"{self.name}-script"]["metadata"]["ckb-contract"]["vm-tests"] = "missing-tests"
        with self.assertRaises(ValueError):
            CONTRACTS.contract_plan(metadata)

    def test_invalid_names_and_overwrites_do_not_change_manifest(self):
        manifest = (self.root / "Cargo.toml").read_text()
        for name in ["../escape", "a/b", "BadName", "x;touch", "a--b", "framework-counter", "framework-vault", "framework-escrow", "framework-vesting", "framework-limit-order"]:
            with self.assertRaises(ValueError):
                SCAFFOLD.create_contract(self.root, name)
            self.assertEqual((self.root / "Cargo.toml").read_text(), manifest)
        target = SCAFFOLD.create_contract(self.root, self.name)
        sentinel = target / "keep.txt"
        sentinel.write_text("keep")
        manifest = (self.root / "Cargo.toml").read_text()
        with self.assertRaises(ValueError):
            SCAFFOLD.create_contract(self.root, self.name)
        self.assertEqual(sentinel.read_text(), "keep")
        self.assertEqual((self.root / "Cargo.toml").read_text(), manifest)

    def test_missing_vm_template_does_not_leave_a_partial_contract(self):
        manifest = (self.root / "Cargo.toml").read_text()
        (self.root / "tests/src/tests.rs").unlink()
        with self.assertRaises(ValueError):
            SCAFFOLD.create_contract(self.root, self.name)
        self.assertFalse((self.root / "contracts" / self.name).exists())
        self.assertEqual((self.root / "Cargo.toml").read_text(), manifest)

    def test_missing_cells_template_does_not_leave_a_partial_contract(self):
        manifest = (self.root / "Cargo.toml").read_text()
        (self.root / "examples/counter/contract/src/cells.rs").unlink()
        with self.assertRaises(ValueError):
            SCAFFOLD.create_contract(self.root, self.name)
        self.assertFalse((self.root / "contracts" / self.name).exists())
        self.assertEqual((self.root / "Cargo.toml").read_text(), manifest)

    def test_existing_workspace_package_names_are_rejected_before_writes(self):
        extra = self.root / "extra-vm-tests"
        (extra / "src").mkdir(parents=True)
        (extra / "src/lib.rs").write_text("")
        (extra / "Cargo.toml").write_text(
            f'[package]\nname = "{self.name}-vm-tests"\nversion = "0.0.0"\n'
        )
        manifest_path = self.root / "Cargo.toml"
        manifest = SCAFFOLD.append_members(manifest_path.read_text(), "members", ["extra-vm-tests"])
        manifest_path.write_text(manifest)
        lock_path = self.root / "Cargo.lock"
        lock_before = lock_path.read_bytes() if lock_path.exists() else None
        for name in ["framework-layout-test", self.name]:
            with self.subTest(name=name):
                with self.assertRaisesRegex(ValueError, "package name already exists"):
                    SCAFFOLD.create_contract(self.root, name)
                self.assertEqual(manifest_path.read_text(), manifest)
                self.assertFalse((self.root / "contracts" / name).exists())
                self.assertEqual(lock_path.read_bytes() if lock_path.exists() else None, lock_before)


if __name__ == "__main__":
    unittest.main()
