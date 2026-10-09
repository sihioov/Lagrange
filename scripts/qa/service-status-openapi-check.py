"""Use the existing exact locked OpenAPI tool; no install or download."""
from pathlib import Path
import hashlib
import json
import os
import subprocess
import sys

root = Path(__file__).resolve().parents[2]
link = root / "node_modules/openapi-typescript"
target = Path("/tmp/lagrange-service-status-openapi-tools/node_modules/openapi-typescript")
if link.exists() or link.is_symlink():
    raise SystemExit("Refusing to replace an existing tool path")
lock = json.loads((root / "package-lock.json").read_text())
locked = lock["packages"]["node_modules/openapi-typescript"]["version"]
actual = json.loads((target / "package.json").read_text())["version"]
if actual != locked:
    raise SystemExit("Existing OpenAPI tool differs from the repository lock")
protected = [root / "apps/api-server/openapi.json", root / "apps/api-server/generated/openapi.ts"]
before = {path: path.read_bytes() for path in protected}
os.symlink(target, link, target_is_directory=True)
try:
    result = subprocess.run(["npm", "run", "openapi:check", "--workspace=@lagrange/api-server"], cwd=root, check=False)
finally:
    if link.is_symlink() and os.readlink(link) == str(target):
        link.unlink()
changed = [path for path in protected if path.read_bytes() != before[path]]
if changed:
    for path in changed:
        path.write_bytes(before[path])
    raise SystemExit("Official check generated differing contract bytes; original read-only files restored; gate failed")
print(json.dumps({"official_check_exit": result.returncode, "locked_tool_version": locked,
    "protected_hashes_unchanged": {str(path.relative_to(root)): hashlib.sha256(before[path]).hexdigest() for path in protected}}))
sys.exit(result.returncode)
