"""GitHub controls for the private, commit-pinned release delivery workflow."""
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import tempfile
from pathlib import Path

SOURCE = "sihioov/Lagrange"
SOURCE_REPOSITORY_ID = "1327711777"
WORKFLOW = ".github/workflows/release-delivery.yml"
RECORD = "delivery.json"
SIGNATURE = "delivery.sigstore.jsonl"
RESERVED = {RECORD, SIGNATURE}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def api(path, *args):
    return json.loads(subprocess.check_output(["gh", "api", "--hostname", "github.com", path, *args]))


def numeric(value):
    require(re.fullmatch(r"[1-9][0-9]{0,19}", str(value)), "invalid run number")
    return int(value)


def check_ci(commit, run_id):
    run = api(f"repos/{SOURCE}/actions/runs/{numeric(run_id)}")
    require(run["head_repository"]["full_name"] == SOURCE and run["head_sha"] == commit
            and run["head_branch"] == "main" and run["event"] in {"push", "workflow_dispatch"}
            and run["path"] == ".github/workflows/ci.yml" and run["status"] == "completed"
            and run["conclusion"] == "success", "a successful main CI run for this exact SHA is required")
    attempt = numeric(run["run_attempt"])
    # A "rerun failed jobs" retains successful jobs from earlier attempts of this run.
    pending = {"required", "paper-image-integration"}
    for number in range(attempt, 0, -1):
        page = api(f"repos/{SOURCE}/actions/runs/{run_id}/attempts/{number}/jobs?per_page=100")
        require(len(page["jobs"]) == page["total_count"], "incomplete CI job listing")
        for name in pending.copy():
            jobs = [job for job in page["jobs"] if job["name"] == name]
            if jobs:
                require(len(jobs) == 1 and jobs[0]["status"] == "completed" and jobs[0]["conclusion"] == "success"
                        and jobs[0]["head_sha"] == commit, f"successful {name} job required for candidate")
                pending.remove(name)
        if not pending:
            break
    require(not pending, "required release job missing")
    return attempt


def private_store(store, *, writing=False):
    require(re.fullmatch(r"[A-Za-z0-9-]+/[A-Za-z0-9_.-]+", store) and store.lower() != SOURCE.lower(),
            "provide a separate private delivery repository")
    repo = api(f"repos/{store}")
    require(repo["full_name"].lower() == store.lower() and repo["private"] is True
            and repo["visibility"] == "private", "destination must be private")
    if writing:
        require(api(f"repos/{store}/immutable-releases")["enabled"] is True,
                "enable immutable releases on the destination before publishing")


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def inventory(directory):
    result = {}
    for path in directory.iterdir():
        require(re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,120}", path.name) and path.is_file()
                and not path.is_symlink() and 0 < path.stat().st_size < 2**31, "invalid release asset")
        result[path.name] = "sha256:" + sha256(path)
    return result


def release_tag(commit, run_id, attempt):
    require(re.fullmatch(r"[0-9a-f]{40}", commit), "invalid source commit")
    return f"lagrange-{commit}-{numeric(run_id)}-{numeric(attempt)}"


def require_bundle(names):
    metadata = {"release-bundle.json", "release.manifest", "d8-artifact.json", "d8-COMPLETE"}
    chunks = sorted(names - metadata)
    require(metadata <= names and chunks and chunks == [
        f"images.tar.zst.part{i:06d}" for i in range(len(chunks))
    ], "unexpected export files or missing archive chunks")


def write_record(directory, commit, ci, build, attempt):
    files = inventory(directory)
    require_bundle(set(files))
    data = {"schema": 1, "source": SOURCE, "commit": commit, "platform": "linux/amd64",
            "tag": release_tag(commit, build, attempt), "ci_run": numeric(ci),
            "ci_attempt": check_ci(commit, ci), "assets": files}
    with (directory / RECORD).open("x") as handle:
        json.dump(data, handle, indent=2, sort_keys=True)


def remote_digests(release):
    files = {}
    for asset in release["assets"]:
        name = asset["name"]
        require(re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,120}", name) and name not in files
                and asset["state"] == "uploaded" and re.fullmatch(r"sha256:[0-9a-f]{64}", asset["digest"] or ""),
                "invalid remote release asset")
        files[name] = asset["digest"]
    require(RESERVED <= files.keys(), "release statement or signature missing")
    require_bundle(set(files) - RESERVED)
    return files


def verify_record(directory, commit, tag, files, trusted_root=None):
    # This remains independently verifiable after Actions logs expire.
    command = ["gh", "attestation", "verify", str(directory / RECORD), "--bundle",
               str(directory / SIGNATURE), "--repo", SOURCE, "--hostname", "github.com",
               "--signer-workflow", SOURCE + "/" + WORKFLOW, "--source-ref", "refs/heads/main",
               "--source-digest", commit, "--deny-self-hosted-runners", "--format", "json",
               "--jq", ".[].verificationResult.signature.certificate.sourceRepositoryIdentifier"]
    if trusted_root is not None:
        command.extend(["--custom-trusted-root", str(trusted_root)])
    ids = subprocess.check_output(command, text=True).splitlines()
    require(ids and set(ids) == {SOURCE_REPOSITORY_ID}, "untrusted signer repository ID")
    require_bundle(set(files) - RESERVED)
    record = json.loads((directory / RECORD).read_text())
    require(record["schema"] == 1 and record["source"] == SOURCE and record["commit"] == commit
            and record["tag"] == tag and record["platform"] == "linux/amd64"
            and record["assets"] == {k: v for k, v in files.items() if k not in RESERVED},
            "release contents differ from authenticated build inventory")


def publish(store, directory, commit, tag):
    private_store(store, writing=True)
    files = inventory(directory)
    verify_record(directory, commit, tag, files)
    # Always create a new draft. Never update an existing tag or replace its assets.
    draft = api(f"repos/{store}/releases", "--method", "POST", "-f", f"tag_name={tag}",
                "-f", f"name={tag}", "-F", "draft=true")
    endpoint = f"repos/{store}/releases/{numeric(draft['id'])}"
    for name in sorted(files):
        subprocess.run(["gh", "release", "upload", tag, str(directory / name), "-R", f"github.com/{store}"],
                       check=True)
    private_store(store, writing=True)
    current = api(endpoint)
    require(current["tag_name"] == tag and current["draft"] is True and remote_digests(current) == files,
            "draft did not receive the exact verified assets")
    final = api(endpoint, "--method", "PATCH", "-F", "draft=false", "-f", "make_latest=false")
    require(final["tag_name"] == tag and final["draft"] is False and final["immutable"] is True
            and remote_digests(final) == files, "published release failed immutable verification")
    print(f"Published immutable release {tag}")


def fetch(store, directory, commit, tag):
    private_store(store)
    release = api(f"repos/{store}/releases/tags/{tag}")
    require(release["tag_name"] == tag and release["draft"] is False and release["immutable"] is True,
            "expected a published immutable release")
    remote = remote_digests(release)
    require(not os.path.lexists(directory), "choose a new download directory")
    with tempfile.TemporaryDirectory(dir=directory.parent) as staging:
        stage = Path(staging)
        downloaded = stage / "received"
        downloaded.mkdir(mode=0o700)
        subprocess.run(["gh", "release", "download", tag, "-R", f"github.com/{store}",
                        "--dir", str(downloaded)], check=True)
        require(inventory(downloaded) == remote, "download digest mismatch")
        verify_record(downloaded, commit, tag, remote)
        # Preserve the signed receipt alongside the payload for later independent verification.
        payload = downloaded / "bundle"
        payload.mkdir(mode=0o700)
        for name in remote.keys() - RESERVED:
            shutil.move(downloaded / name, payload / name)
        downloaded.rename(directory)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="cmd", required=True)
    gate = commands.add_parser("check-ci")
    gate.add_argument("commit")
    gate.add_argument("ci_run")
    store = commands.add_parser("check-store")
    store.add_argument("store")
    for command in ("record", "publish", "fetch", "verify-offline"):
        sub = commands.add_parser(command)
        sub.add_argument("directory", type=Path)
        sub.add_argument("commit")
        sub.add_argument("build_run")
        sub.add_argument("build_attempt")
        if command == "verify-offline":
            sub.add_argument("trusted_root", type=Path)
        else:
            sub.add_argument("ci_run" if command == "record" else "store")
    args = parser.parse_args()
    os.umask(0o077)
    if args.cmd == "check-store":
        private_store(args.store, writing=True)
    elif args.cmd == "check-ci":
        check_ci(args.commit, args.ci_run)
    elif args.cmd == "record":
        write_record(args.directory, args.commit, args.ci_run, args.build_run, args.build_attempt)
    else:
        tag = release_tag(args.commit, args.build_run, args.build_attempt)
        if args.cmd == "verify-offline":
            files = inventory(args.directory / "bundle")
            require(files and not RESERVED.intersection(files), "invalid offline bundle")
            verify_record(args.directory, args.commit, tag, files, args.trusted_root)
            print("Verified offline release " + tag)
            return
        operation = publish if args.cmd == "publish" else fetch
        operation(args.store, args.directory, args.commit, tag)


if __name__ == "__main__":
    main()
