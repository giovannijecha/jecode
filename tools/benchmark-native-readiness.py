#!/usr/bin/env python3
"""Run the staged pilot's native runtime in an account-free, offline sandbox."""

import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import tomllib
from datetime import datetime, timezone
from importlib.metadata import version
from pathlib import Path

LAB = Path("/var/lib/jecode-benchmark")
RUNTIME = Path("/opt/jecode-benchmark")
DOCKER = ["docker", "--host", "unix:///run/jecode-bench-quota/docker.sock"]
PROFILES = {"jecode-openai", "jecode-anthropic", "codex-openai", "claude-anthropic"}
HOME_TARGETS = {"/benchmark-home", "/benchmark-auth", "/benchmark-home/jecode/accounts"}
OS_TARGETS = {
    "/usr/lib/x86_64-linux-gnu/libcrypto.so.3",
    "/usr/lib/x86_64-linux-gnu/libssl.so.3",
    "/etc/ssl/certs/ca-certificates.crt",
    "/usr/lib/ssl/cert.pem",
}
CA_OVERRIDES = {
    "SSL_CERT_FILE", "SSL_CERT_DIR", "CURL_CA_BUNDLE", "REQUESTS_CA_BUNDLE",
    "NODE_EXTRA_CA_CERTS", "NODE_TLS_REJECT_UNAUTHORIZED",
}
BRIDGE = "/opt/benchmark/bridge"
NATIVE = "/opt/benchmark/native"
CODEX_HOST = "/opt/benchmark/codex-code-mode-host"
CRYPTO = "/opt/benchmark/openai_crypto"
VENV = "/opt/harbor-acp-venv"
PROBE = "/opt/benchmark/readiness.py"
SHA_ABC = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"


class ReadinessError(Exception):
    pass


def require(condition, message):
    if not condition:
        raise ReadinessError(message)


def sha256(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def digest(path):
    if path.is_file():
        return sha256(path)
    tree = hashlib.sha256()
    for entry in sorted(path.rglob("*")):
        if entry.is_symlink():
            kind, value = "link", os.readlink(entry)
        elif entry.is_dir():
            kind, value = "directory", ""
        else:
            require(entry.is_file(), "Unexpected special file in the ACP runtime.")
            kind, value = "file", sha256(entry)
        item = [entry.relative_to(path).as_posix(), kind, value]
        tree.update(json.dumps(item).encode() + b"\n")
    return tree.hexdigest()


def runtime_mounts(environment, harness):
    require(not CA_OVERRIDES.intersection(environment.get("env", {})),
            "Staged CA environment overrides prevent a default trust check.")
    required_roots = {
        BRIDGE: (LAB / "runner-builds",),
        NATIVE: (LAB / "builds",) if harness == "jecode" else (RUNTIME / "agents",),
        VENV: (RUNTIME,),
    }
    if harness == "codex":
        required_roots[CODEX_HOST] = (RUNTIME / "agents",)
    mounts = {}
    python_tree = None
    for binding in environment["mounts"]:
        target = binding["target"]
        if target in HOME_TARGETS:
            continue
        path = Path(binding["source"])
        roots = required_roots.get(target)
        if target in OS_TARGETS:
            roots = (RUNTIME,)
        if re.fullmatch(r"/opt/jecode-benchmark/acp-runtime-[a-z0-9-]+/python", target):
            require(python_tree is None and str(path) == target,
                    "The staged ACP Python runtime is ambiguous.")
            python_tree = path
            roots = (RUNTIME,)
        require(roots and any(path.is_relative_to(root) for root in roots)
                and re.fullmatch(r"/[a-zA-Z0-9_./-]+", str(path))
                and path == path.resolve(strict=True) and target not in mounts
                and binding["type"] == "bind" and binding.get("read_only") is True,
                "Unexpected or writable runtime bind in the staged configuration.")
        mounts[target] = path
    require(required_roots.keys() <= mounts.keys() and python_tree is not None,
            "The staged configuration omits part of the native or ACP runtime.")
    require(mounts[VENV] == python_tree.with_name("venv")
            and set(target for target, path in mounts.items() if path.is_dir()) ==
            {VENV, str(python_tree)},
            "Staged runtime directories differ from the ACP Python/venv pair.")
    return mounts


def staged_runtime(run_id, mode, report):
    root = LAB / "pilot" / run_id
    checkpoint_path = root / "checkpoint.json"
    checkpoint = checkpoint_path.read_bytes()
    record = json.loads(checkpoint)
    require(record["run_id"] == run_id and record["profile"] in PROFILES
            and re.fullmatch(r"[a-z0-9][a-z0-9-]*", record["task_id"])
            and record["scope"] in {"official_task_calibration", "official_task_round"},
            "Unexpected pilot checkpoint identity.")
    if mode == "run":
        require(record["phase"] == "staged" and not (LAB / "pilot/jobs" / run_id).exists(),
                "Pilot must be staged and unused before launch.")
    report.update(task_id=record["task_id"], profile=record["profile"],
                  pilot_scope=record["scope"], pilot_phase=record["phase"],
                  with_os_runtime=bool(record.get("os_runtime_manifest_sha256")))

    config_path = root / "config.json"
    task_path = root / "task/task.toml"
    config_bytes, task_bytes = config_path.read_bytes(), task_path.read_bytes()
    report.update(checkpoint_sha256=hashlib.sha256(checkpoint).hexdigest(),
                  config_sha256=hashlib.sha256(config_bytes).hexdigest(),
                  task_sha256=hashlib.sha256(task_bytes).hexdigest())
    require(report["config_sha256"] == record["config_sha256"]
            and report["task_sha256"] == record["modified_task_toml_sha256"],
            "Staged pilot configuration or task metadata changed.")
    config = json.loads(config_bytes)
    task = tomllib.loads(task_bytes.decode())["environment"]
    image = task["docker_image"]
    uid = record["agent_uid"]
    environment = config["environment"]
    local = config["agents"][0]["kwargs"]["registry_entry"]["distribution"]["local"]
    harness = record["profile"].split("-")[0]
    require(image == record["agent_image"] and re.fullmatch(r"sha256:[0-9a-f]{64}", image)
            and type(uid) is int and uid >= 0 and config["job_name"] == run_id
            and environment["type"] == "docker" and config["agents"][0]["name"] == "acp"
            and local["cmd"] == BRIDGE and local["args"][:4] == ["--harness", harness, "--executable", NATIVE],
            "Staged image, user or agent runtime differs from the pilot checkpoint.")
    report.update(image=image, expected_uid=uid)
    mounts = runtime_mounts(environment, harness)
    if harness == "jecode":
        native_bytes = mounts[NATIVE].read_bytes()
        candidates = sorted(mounts[NATIVE].parent.glob("build/jecode-*/out/jecode-openai_crypto"))
        for candidate in candidates:
            if candidate.is_file() and candidate == candidate.resolve() and candidate.read_bytes() in native_bytes:
                mounts[CRYPTO] = candidate
                break
        require(CRYPTO in mounts, "The staged Jecode build lacks its embedded crypto helper artifact.")
    mounts[PROBE] = Path(__file__).resolve()
    pins = {BRIDGE: record["acp_sha256"]}
    if harness == "jecode":
        pins[NATIVE] = record["jecode_sha256"]
    if harness == "codex":
        pins[CODEX_HOST] = record["codex_code_mode_host_sha256"]
    os_files = (record.get("os_runtime_manifest") or {}).get("files", [])
    require({file["target"] for file in os_files} == OS_TARGETS.intersection(mounts)
            and bool(os_files) == report["with_os_runtime"],
            "Staged OS runtime mounts differ from their manifest.")
    for file in os_files:
        require(mounts[file["target"]] == Path(file["source"]), "Staged OS runtime source differs.")
        pins[file["target"]] = file["sha256"]
    report["os_runtime_manifest_sha256"] = record.get("os_runtime_manifest_sha256")
    report["runtime"] = []
    for target, path in mounts.items():
        actual = digest(path)
        report["runtime"].append({"target": target, "sha256": actual,
                                  "kind": "tree" if path.is_dir() else "file"})
        require(target not in pins or pins[target] == actual, f"Runtime digest changed at {target}.")
    return mounts, task


def invoke(arguments, timeout=15, **options):
    try:
        return subprocess.run(arguments, capture_output=True, timeout=timeout, **options)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ReadinessError(f"Process could not complete ({type(error).__name__}).") from None


def check_container(report, mounts, task):
    name = "jecode-readiness-" + Path(report["receipt"]).parent.name
    report["container_name"] = name
    arguments = [
        "create", "-i", "--name", name, "--pull=never", "--network=none", "--read-only",
        "--cap-drop=ALL", "--security-opt=no-new-privileges",
        "--tmpfs", "/tmp:rw,exec,nosuid,nodev,size=64m",
        "--cpus", str(task["cpus"]), "--memory", f'{task["memory_mb"]}m',
        "--label", "jecode.owner=benchmark-native-readiness",
        "--entrypoint", f"{VENV}/bin/python",
    ]
    for target, source in mounts.items():
        arguments += ["--mount", f"type=bind,src={source},dst={target},readonly"]
    arguments += [report["image"], "-I", "-B", PROBE, "--inside"]
    creation = invoke(DOCKER + arguments)
    container = creation.stdout.decode(errors="replace").strip()
    require(creation.returncode == 0 and re.fullmatch(r"[0-9a-f]{64}", container),
            "Could not create the offline container from the local quota-daemon image.")
    report["container_id"] = container
    try:
        spec = {key: report[key] for key in ("expected_uid", "profile", "runtime")}
        started = invoke(DOCKER + ["start", "-ai", container], timeout=60,
                         input=json.dumps(spec).encode())
        report["container_exit"] = started.returncode
        try:
            report["observed"] = json.loads(started.stdout)
        except ValueError:
            raise ReadinessError("The runtime did not return a probe result as the image user.") from None
        require(started.returncode == 0 and report["observed"]["passed"] is True,
                "One or more functional startup checks failed.")
    finally:
        removed = invoke(DOCKER + ["rm", "-fv", container])
        report["container_removed"] = removed.returncode == 0
    require(report["container_removed"], "Readiness container could not be retired by exact ID.")


def inside(spec):
    checks = {}

    def run_check(name, operation):
        try:
            checks[name] = {"passed": True, **operation()}
        except ReadinessError as error:
            checks[name] = {"passed": False, "error": str(error)}
        except (OSError, ImportError, ValueError, KeyError) as error:
            checks[name] = {"passed": False, "error": type(error).__name__}

    def identity():
        require(os.geteuid() == spec["expected_uid"], "Effective image UID differs.")
        for item in spec["runtime"]:
            require(digest(Path(item["target"])) == item["sha256"],
                    f'Mounted bytes differ at {item["target"]}.')
        return {"runtime_digests_verified": True}

    scratch = Path(tempfile.mkdtemp(dir="/tmp"))
    home = scratch / "home"
    home.mkdir()
    child_env = {
        "HOME": str(home), "JECODE_HOME": str(home), "CODEX_HOME": str(home),
        "CLAUDE_CONFIG_DIR": str(home), "TMPDIR": str(scratch),
        "PATH": "/usr/local/bin:/usr/bin:/bin", "DISABLE_TELEMETRY": "1", "DISABLE_UPDATES": "1",
    }

    def command(path, *args, data=b""):
        result = invoke([path, *args], env=child_env, input=data)
        if result.returncode:
            require(result.stderr.strip() != b"OS libcrypto unavailable", "OS libcrypto unavailable.")
            raise ReadinessError(f"Native process exited {result.returncode}.")
        return result.stdout

    def version_check():
        require(command(NATIVE, "--version").strip(), "Empty native version response.")
        return {}

    def code_host():
        require(command(CODEX_HOST, "--help").strip(), "Empty code-mode host help.")
        command(CODEX_HOST)
        return {}

    def acp_initialize():
        import acp

        harness, provider = spec["profile"].split("-")
        request = {"jsonrpc": "2.0", "id": 1, "method": "initialize",
                   "params": {"protocolVersion": 1, "clientCapabilities": {}}}
        reply = json.loads(command(
            BRIDGE, "--harness", harness, "--provider", provider, "--model", "benchmark-offline",
            "--executable", NATIVE, "--home", str(home), "--artifacts", str(scratch / "artifacts"),
            data=json.dumps(request).encode() + b"\n",
        ))
        require(reply["id"] == 1 and reply["jsonrpc"] == "2.0"
                and reply["result"]["protocolVersion"] == 1 and not reply.get("error"),
                "ACP initialization did not negotiate version 1.")
        return {"acp_sdk": version("agent-client-protocol"), "protocol": 1}

    def crypto():
        text = b"abc"
        response = command(CRYPTO, data=b"h" + len(text).to_bytes(4, "big") + text)
        require(response.hex() == SHA_ABC, "Native SHA-256 disagrees with the synthetic vector.")
        count = 32
        random_bytes = command(CRYPTO, data=b"r" + b"\x00\x00\x00\x04" + count.to_bytes(4, "big"))
        require(len(random_bytes) == count, "Native random output length differs.")
        return {"sha256_input": text.decode(), "sha256": SHA_ABC, "random_bytes": count}

    def default_trust():
        require(not CA_OVERRIDES.intersection(os.environ),
                "Image CA environment overrides prevent a default trust check.")
        try:
            import ssl
        except ImportError:
            raise ReadinessError("The staged TLS runtime could not load its OS OpenSSL libraries.") from None

        paths = ssl.get_default_verify_paths()
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        context.set_default_verify_paths()
        count = len(context.get_ca_certs())
        result = {"openssl": ssl.OPENSSL_VERSION, "default_ca_file": paths.openssl_cafile,
                  "default_ca_directory": paths.openssl_capath, "loaded_ca_count": count,
                  "ca_sha256": sha256(paths.cafile) if paths.cafile else None}
        if not count:
            return {**result, "passed": False, "error": "OpenSSL default paths loaded no CA certificates."}
        return result

    run_check("identity", identity)
    run_check("native_version", version_check)
    run_check("acp_initialize", acp_initialize)
    if spec["profile"] == "codex-openai":
        run_check("code_mode_host", code_host)
    if spec["profile"].startswith("jecode-"):
        run_check("crypto", crypto)
    if spec["profile"] != "claude-anthropic":
        run_check("default_trust", default_trust)
    passed = all(result["passed"] for result in checks.values())
    print(json.dumps({"uid": os.geteuid(), "gid": os.getegid(), "checks": checks, "passed": passed}))
    return 0 if passed else 1


def main(run_id, mode):
    require(re.fullmatch(r"[a-z0-9][a-z0-9-]{0,59}", run_id) and mode in {"probe", "run"},
            "Invalid readiness ID or mode.")
    evidence = LAB / "native-readiness"
    evidence.mkdir(mode=0o700, exist_ok=True)
    receipt = Path(tempfile.mkdtemp(prefix=f"{run_id}-", dir=evidence)) / "receipt.json"
    report = {
        "schema_version": 1, "scope": "offline_native_readiness", "run_id": run_id,
        "mode": mode, "created_at_utc": datetime.now(timezone.utc).isoformat(),
        "docker_host": DOCKER[-1], "network": "none", "public_https": "not_run",
        "account_access": False, "model_requests": 0, "receipt": str(receipt),
        "verdict": "failed",
    }
    try:
        mounts, task = staged_runtime(run_id, mode, report)
        check_container(report, mounts, task)
        if mode == "run":
            root = LAB / "pilot" / run_id
            staged = {"checkpoint_sha256": "checkpoint.json", "config_sha256": "config.json",
                      "task_sha256": "task/task.toml"}
            require(all(sha256(root / path) == report[key] for key, path in staged.items()),
                    "Pilot staging changed during the offline check.")
        report["verdict"] = "passed"
    except ReadinessError as error:
        report["error"] = str(error)
    except (OSError, TypeError, ValueError, KeyError, IndexError) as error:
        report["error"] = f"Could not verify the staged pilot runtime ({type(error).__name__})."
    with receipt.open("x", encoding="utf-8") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")
    report["receipt_sha256"] = sha256(receipt)
    print(json.dumps(report))
    return 0 if report["verdict"] == "passed" else 1


if __name__ == "__main__":
    if sys.argv[1:] == ["--inside"]:
        sys.exit(inside(json.load(sys.stdin)))
    sys.exit(main(*sys.argv[1:]))
