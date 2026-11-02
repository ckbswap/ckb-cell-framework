#!/usr/bin/env python3
"""Cache the pinned official xUDT binary; no wallet or signing is involved."""
import argparse
import hashlib
import json
from pathlib import Path
import urllib.request


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--from-file", type=Path, help="Import a local official xUDT binary")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    target = root / "vendor" / "xudt"
    expected = "50bd8d6680b8b9cf98b73f3c08faf8b2a21914311954118ad6609be6e78a1b95"
    if args.from_file:
        data = args.from_file.read_bytes()
    elif target.is_file():
        data = target.read_bytes()
    else:
        body = json.dumps({
            "id": 1, "jsonrpc": "2.0", "method": "get_transaction",
            "params": ["0xbf6fb538763efec2a70a6a3dcb7242787087e1030c4e7d86585bc63a9d337f5f"],
        }).encode()
        request = urllib.request.Request(
            "https://testnet.ckb.dev", data=body,
            headers={"Content-Type": "application/json"},
        )
        with urllib.request.urlopen(request, timeout=30) as response:
            result = json.load(response)
        if result.get("error"):
            raise SystemExit("RPC could not provide the official xUDT deployment")
        data = bytes.fromhex(result["result"]["transaction"]["outputs_data"][0][2:])
    actual = hashlib.blake2b(data, digest_size=32, person=b"ckb-default-hash").hexdigest()
    if actual != expected:
        raise SystemExit(f"Official xUDT hash mismatch: {actual}")
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(data)
    print(f"Verified official xUDT: {len(data)} bytes")


if __name__ == "__main__":
    main()
