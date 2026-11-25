#!/usr/bin/env python3
"""Use a cached pinned Nginx to exercise a real, isolated API address change."""
import hashlib
import json
from pathlib import Path
import re
import runpy
import subprocess
import tempfile
import time
import uuid

DOCKER = ["/usr/bin/docker", "--host", "unix:///var/run/docker.sock"]
BASE = Path(__file__).resolve().parents[1]
LABEL = "io.lagrange.nginx-api-replacement"


def require(value, code):
    if not value:
        raise RuntimeError(code)


def command(argv, code, *, timeout=7):
    result = subprocess.run(argv, stdin=subprocess.DEVNULL, capture_output=True, timeout=timeout, check=False)
    require(result.returncode == 0, code)
    return result.stdout


def docker(argv, code):
    return command(DOCKER + argv, code)


def inspect(kind, value):
    facts = json.loads(docker([kind, "inspect", value], "INSPECT"))
    require(len(facts) == 1 and isinstance(facts[0], dict), "INSPECT")
    return facts[0]


def fixture():
    compose = (BASE / "compose/compose.yml").read_text()
    pins = re.findall(r"(?m)^  reverse-proxy:\n    image: (nginxinc/nginx-unprivileged@sha256:[0-9a-f]{64}) ", compose)
    require(len(pins) == 1, "PROXY_IMAGE_PIN")
    image = inspect("image", pins[0])
    require(pins[0] in image["RepoDigests"], "PROXY_IMAGE_PIN")
    nginx = BASE / "nginx/nginx.conf"
    parse_probe = runpy.run_path(str(BASE.parent / "scripts/ops/lib/owner-beta-paper-serving.py"))["routed_session_response"]
    nonce = uuid.uuid4().hex
    name = "lagrange-api-replacement-" + nonce
    expected_names = {name + "-" + role for role in ("web", "first", "proxy", "replacement")}
    with tempfile.TemporaryDirectory(prefix=name) as temp:
        directory = Path(temp)
        cert, key = directory / "cert.pem", directory / "key.pem"
        openssl = ["/usr/bin/openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
                   "-subj", "/CN=isolated.fixture", "-days", "1", "-keyout", str(key), "-out", str(cert)]
        command(openssl, "FIXTURE_CERTIFICATE")
        for path in (cert, key):
            path.chmod(0o444)
        for phase, port in (("web", 3000), ("first", 8080), ("replacement", 8080)):
            # Fixed public fixture replies expose unchanged URI and scheme.
            text = ("worker_processes 1; pid /tmp/nginx.pid; events {} http {"
                    " server { listen " + str(port) + ";"
                    " location = /api/v1/auth/session { default_type application/json; return 401 '{}'; }"
                    ' location / { return 200 "' + phase + '|$request_uri|$http_x_forwarded_proto"; }} }')
            (directory / (phase + ".conf")).write_text(text, encoding="ascii")
        network = docker(["network", "create", "--internal", "--label", LABEL + "=" + nonce, name], "NETWORK").decode().strip()
        try:
            binding = inspect("network", network)
            require(binding["Name"] == name and binding["Internal"] is True and
                    binding["Labels"].get(LABEL) == nonce, "NETWORK")
            def run(role, alias, config, mounts=()):
                args = ["create", "--name", name + "-" + role, "--label", LABEL + "=" + nonce,
                        "--pull", "never", "--network", network, "--network-alias", alias,
                        "--user", "101:101", "--read-only", "--cap-drop", "ALL",
                        "--security-opt", "no-new-privileges:true", "--cpus", "0.25", "--memory", "64m",
                        "--pids-limit", "32", "--tmpfs", "/tmp:rw,nosuid,nodev,mode=1777",
                        "--tmpfs", "/var/cache/nginx:rw,nosuid,nodev,uid=101,gid=101,mode=0750",
                        "--entrypoint", "/usr/sbin/nginx",
                        "--mount", "type=bind,src=" + str(config) + ",dst=/etc/nginx/nginx.conf,readonly"]
                for path, target in mounts:
                    args += ["--mount", "type=bind,src=" + str(path) + ",dst=" + target + ",readonly"]
                args += [image["Id"], "-g", "daemon off;"]
                cid = docker(args, "CREATE").decode().strip()
                require(re.fullmatch(r"[a-f0-9]{64}", cid), "CREATE")
                item = inspect("container", cid)
                require(item["Config"]["Labels"].get(LABEL) == nonce and
                        item["Image"] == image["Id"] and item["HostConfig"]["NetworkMode"] == network and
                        not item["HostConfig"]["PortBindings"], "CONTAINER_BOUNDARY")
                docker(["start", cid], "START")
                return cid
            def generation(cid):
                value = inspect("container", cid)
                require(value["State"]["Running"] and value["RestartCount"] == 0, "GENERATION")
                return (value["Id"], value["State"]["StartedAt"],
                        value["NetworkSettings"]["Networks"][name]["IPAddress"])
            def wait_for(cid, target):
                deadline = time.monotonic() + 13
                while time.monotonic() < deadline:
                    try:
                        body = docker(["exec", "--user", "101:101", cid, "/usr/bin/wget",
                            "--no-check-certificate", "-T", "2", "-qO-", target[0]], "REQUEST")
                        if body == target[1]:
                            return
                    except RuntimeError:
                        pass
                    time.sleep(.25)
                raise RuntimeError("ROUTE")
            run("web", "web", directory / "web.conf")
            old = run("first", "api-server", directory / "first.conf")
            proxy = run("proxy", "proxy", nginx, ((cert, "/run/secrets/lagrange_tls_cert"),
                                                  (key, "/run/secrets/lagrange_tls_key")))
            prior_proxy, prior_api = generation(proxy), generation(old)
            url = "https://127.0.0.1:8443/api/replace-fixture?check=1"
            wait_for(proxy, (url, b"first|/api/replace-fixture?check=1|https"))
            replacement = run("replacement", "api-server", directory / "replacement.conf")
            after_api = generation(replacement)
            require(prior_api[0] != after_api[0] and prior_api[2] != after_api[2], "ADDRESS_CHANGED")
            docker(["container", "rm", "--force", old], "REMOVE_OLD")
            wait_for(proxy, (url, b"replacement|/api/replace-fixture?check=1|https"))
            require(generation(proxy) == prior_proxy, "PROXY_GENERATION_CHANGED")
            # Production probe treats a fixed unauthenticated API route's
            # JSON 401 as positive transport, without sending credentials.
            response = docker(["exec", "--user", "101:101", proxy, "/bin/sh", "-c",
                "/usr/bin/wget -S -T 2 -Y off -O /dev/null --no-check-certificate "
                "https://127.0.0.1:8443/api/v1/auth/session 2>&1; exit 0"], "AUTH_PROBE")
            require(parse_probe(response.decode("ascii")), "AUTH_PROBE")
        finally:
            ids = docker(["ps", "-aq", "--no-trunc", "--filter", "label=" + LABEL + "=" + nonce], "CLEANUP").decode().split()
            require(len(ids) <= len(expected_names), "CLEANUP_SCOPE")
            for cid in ids:
                item = inspect("container", cid)
                require(item["Id"] == cid and item["Config"]["Labels"].get(LABEL) == nonce
                        and item["Name"].lstrip("/") in expected_names and item["Image"] == image["Id"],
                        "CLEANUP_SCOPE")
                docker(["container", "rm", "--force", cid], "CLEANUP")
            current = inspect("network", network)
            require(current["Id"] == network and current["Name"] == name and current["Labels"].get(LABEL) == nonce,
                    "CLEANUP_SCOPE")
            docker(["network", "rm", network], "CLEANUP")
            require(not docker(["ps", "-aq", "--filter", "label=" + LABEL + "=" + nonce], "CLEANUP"), "CLEANUP")
    print("NGINX_API_REPLACEMENT: PASS changed_address unchanged_proxy uri_and_scheme preserved unauthenticated_api_401 cleanup")
    print("NGINX_CONFIG_SHA256 " + hashlib.sha256(nginx.read_bytes()).hexdigest())


if __name__ == "__main__":
    try:
        fixture()
    except (RuntimeError, OSError, KeyError, ValueError, subprocess.TimeoutExpired) as error:
        label = str(error)
        if not re.fullmatch(r"[A-Z_]+", label):
            label = type(error).__name__
        raise SystemExit("NGINX_API_REPLACEMENT: " + label) from None
