#!/usr/bin/env python3
"""Copy only a newly built QA binary's resolved loader and libraries."""

from __future__ import annotations

import argparse
import os
import re
import shutil
import stat
import subprocess
from pathlib import Path

TASK_ROOT = re.compile(r"^lagrange-service-status-wp5-[A-Za-z0-9][A-Za-z0-9._-]*$")
INTERPRETER = re.compile(r"Requesting program interpreter:\s*(/[^]]+)\]")
LIBRARY = re.compile(r"^\s*(\S+)\s+=>\s+(\S+)\s+\(")
DIRECT_LIBRARY = re.compile(r"^\s*(/\S+)\s+\(")
SAFE_NAME = re.compile(r"^[A-Za-z0-9_.+-]+$")


def regular_resolved(path_text: str) -> Path:
    path = Path(path_text)
    if not path.is_absolute():
        raise ValueError(f"dependency path is not absolute: {path_text}")
    resolved = path.resolve(strict=True)
    if not resolved.is_absolute() or not stat.S_ISREG(resolved.stat().st_mode):
        raise ValueError(f"dependency is not a regular file: {path_text}")
    return resolved


def run(command: list[str]) -> str:
    result = subprocess.run(command, check=True, capture_output=True, text=True, timeout=30)
    return result.stdout


def prepare(binary_text: str, destination_text: str) -> tuple[str, list[tuple[str, Path]]]:
    binary = regular_resolved(binary_text)
    expected_binary = Path(__file__).resolve().parents[2] / "target/debug/examples/service-status-browser-harness"
    if binary != expected_binary.resolve(strict=True):
        raise ValueError("only the built service-status QA example may be bundled")
    destination = Path(destination_text)
    if not destination.is_absolute():
        raise ValueError("loader bundle must be inside /tmp/<unique-task-root>/actual")
    task_root = destination.parent.parent
    if task_root.parent != Path("/tmp") or not TASK_ROOT.fullmatch(task_root.name) or destination.parent.name != "actual":
        raise ValueError("loader bundle is outside the declared WP-5 task tree")
    if task_root.is_symlink() or destination.parent.is_symlink() or destination.is_symlink():
        raise ValueError("loader bundle path must not contain symlinks")
    if os.stat(task_root).st_uid != os.getuid() or stat.S_IMODE(os.stat(task_root).st_mode) != 0o700:
        raise ValueError("loader bundle task root must be user-owned and mode 0700")
    if os.stat(destination.parent).st_uid != os.getuid() or stat.S_IMODE(os.stat(destination.parent).st_mode) != 0o700:
        raise ValueError("loader bundle actual directory must be user-owned and mode 0700")
    if destination.exists():
        raise FileExistsError("loader bundle destination already exists")

    program_headers = run(["readelf", "-lW", str(binary)])
    interpreter_matches = INTERPRETER.findall(program_headers)
    if len(interpreter_matches) != 1:
        raise ValueError("binary has no unambiguous ELF program interpreter")
    interpreter_path = interpreter_matches[0]
    interpreter = regular_resolved(interpreter_path)
    interpreter_name = Path(interpreter_path).name
    if not SAFE_NAME.fullmatch(interpreter_name):
        raise ValueError("ELF interpreter has an unsafe basename")

    dependency_output = run(["ldd", str(binary)])
    if "not found" in dependency_output or "statically linked" in dependency_output:
        raise ValueError("binary dependencies are unresolved or the binary is not dynamically linked")
    dependencies: dict[str, Path] = {interpreter_name: interpreter}
    for line in dependency_output.splitlines():
        match = LIBRARY.match(line)
        if match:
            required_name, source_text = match.groups()
            if source_text == "not":
                raise ValueError("binary has an unresolved shared library")
        else:
            direct = DIRECT_LIBRARY.match(line)
            if direct is None:
                if "linux-vdso" in line or not line.strip():
                    continue
                if "=>" in line:
                    raise ValueError(f"unrecognized or ambiguous dependency line: {line.strip()}")
                continue
            source_text = direct.group(1)
            required_name = Path(source_text).name
        if not SAFE_NAME.fullmatch(required_name):
            raise ValueError(f"unsafe dependency name: {required_name}")
        source = regular_resolved(source_text)
        prior = dependencies.get(required_name)
        if prior is not None and prior != source:
            raise ValueError(f"dependency name resolves ambiguously: {required_name}")
        dependencies[required_name] = source

    destination.mkdir(mode=0o700)
    library_dir = destination / "lib"
    library_dir.mkdir(mode=0o755)
    copied: list[tuple[str, Path]] = []
    try:
        for name, source in sorted(dependencies.items()):
            target_dir = destination if name == interpreter_name else library_dir
            target = target_dir / name
            if target.exists() or target.is_symlink():
                raise ValueError(f"duplicate loader artifact name: {name}")
            shutil.copyfile(source, target, follow_symlinks=False)
            target.chmod(0o755 if name == interpreter_name else 0o644)
            copied.append((name, source))
        destination.chmod(0o755)
    except BaseException:
        shutil.rmtree(destination)
        raise
    return interpreter_name, copied


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("binary")
    parser.add_argument("destination")
    args = parser.parse_args()
    loader, dependencies = prepare(args.binary, args.destination)
    print(f"loader={loader}")
    for name, source in dependencies:
        print(f"dependency={name} source={source}")


if __name__ == "__main__":
    main()
