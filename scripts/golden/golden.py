#!/usr/bin/env python3
"""golden.py - CLI for the Lagrange Station golden-manifest harness.

Subcommands
-----------
  hash     <file> [--algo sha256]   content-addressed hash of one file
  generate <config.json> [-o manifest.json] [--code-override <commit>]
                                   build a GoldenManifest over fixtures+artifacts
  verify   <manifest.json> [--report <json>]
                                   recompute hashes, field-level diff on drift;
                                   exit 0 unchanged, 1 on any drift
  evidence <manifest.json> -o <file>
                                   write a sanitized evidence text

Exit codes: 0 success; 1 verification drift or runtime error; 2 usage error.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import golden_lib as gl


def _die(message: str, code: int = 2) -> None:
    print(f"golden: error: {message}", file=sys.stderr)
    sys.exit(code)


def cmd_hash(args: argparse.Namespace) -> None:
    if args.algo != gl.ALGO:
        _die(f"unsupported algorithm '{args.algo}' (only {gl.ALGO})")
    digest, meta = gl.hash_file(Path(args.file))
    kind = meta["kind"]
    note = f" ({meta['note']})" if meta.get("note") else ""
    print(f"{digest}  {args.file}  [{kind}]{note}")


def cmd_generate(args: argparse.Namespace) -> None:
    config_path = Path(args.config)
    base_dir = config_path.resolve().parent
    config = gl.load_json_file(config_path)
    if not isinstance(config, dict) or "golden_id" not in config:
        _die(f"{config_path} is not a golden generation config")
    if args.code_override:
        code = {"commit": args.code_override, "tree": args.code_override}
    else:
        code = gl.resolve_code(base_dir)
    out_path = Path(args.output) if args.output else base_dir / "manifest.json"
    manifest = gl.manifest_from_config(config, base_dir, code, output_dir=out_path.resolve().parent)
    out_path.write_bytes(gl.serialize_manifest(manifest))
    print(
        f"wrote {out_path} golden_id={manifest['golden_id']} "
        f"code.commit={code['commit'][:12]} "
        f"config.hash={manifest['versions']['config']['hash'][:20]}... "
        f"({len(gl.serialize_manifest(manifest))} bytes, deterministic)"
    )


def cmd_verify(args: argparse.Namespace) -> None:
    manifest_path = Path(args.manifest)
    base_dir = manifest_path.resolve().parent
    try:
        manifest = gl.load_manifest(manifest_path)
        report = gl.verify_manifest(manifest, base_dir)
    except gl.GoldenManifestError as exc:
        _die(str(exc))
    sys.stdout.write(gl.render_report(report))
    if args.report:
        Path(args.report).write_text(
            json.dumps(report.to_dict(), indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
    sys.exit(0 if report.ok else 1)


def cmd_evidence(args: argparse.Namespace) -> None:
    manifest_path = Path(args.manifest)
    base_dir = manifest_path.resolve().parent
    try:
        manifest = gl.load_manifest(manifest_path)
        report = gl.verify_manifest(manifest, base_dir)
    except gl.GoldenManifestError as exc:
        _die(str(exc))
    gl.write_evidence(manifest, report, Path(args.output))
    sys.exit(0 if report.ok else 1)


def main(argv: list[str] | None = None) -> None:
    parser = argparse.ArgumentParser(
        prog="golden", description="Lagrange Station golden-manifest harness"
    )
    sub = parser.add_subparsers(dest="command", required=True)

    p_hash = sub.add_parser("hash", help="content hash of one file")
    p_hash.add_argument("file")
    p_hash.add_argument("--algo", default=gl.ALGO)
    p_hash.set_defaults(func=cmd_hash)

    p_gen = sub.add_parser("generate", help="build a GoldenManifest from a config")
    p_gen.add_argument("config")
    p_gen.add_argument("-o", "--output", help="output manifest path (default: next to config)")
    p_gen.add_argument(
        "--code-override",
        help="pin code commit/tree (hermetic runs; default: resolve from git)",
    )
    p_gen.set_defaults(func=cmd_generate)

    p_ver = sub.add_parser("verify", help="verify artifacts/fixtures against a manifest")
    p_ver.add_argument("manifest")
    p_ver.add_argument("--report", help="also write a machine-readable JSON report")
    p_ver.set_defaults(func=cmd_verify)

    p_ev = sub.add_parser("evidence", help="write a sanitized evidence text")
    p_ev.add_argument("manifest")
    p_ev.add_argument("-o", "--output", required=True)
    p_ev.set_defaults(func=cmd_evidence)

    args = parser.parse_args(argv)
    args.func(args)


if __name__ == "__main__":
    main()
