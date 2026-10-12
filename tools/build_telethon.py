"""Build an explicit patched Telethon wheel; never modify an installed package."""

from __future__ import annotations

import argparse
import base64
import csv
import datetime as dt
import hashlib
import io
import json
import logging
import os
from pathlib import Path, PurePosixPath
import sys
import tempfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
CONFIG = ROOT / "prerequisites" / "telethon"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def read_archive(path, config):
    if path.stat().st_size > config["max_download_bytes"]:
        raise ValueError("upstream archive exceeds its byte ceiling")
    data = path.read_bytes()
    if digest(data) != config["sha256"]:
        raise ValueError("upstream archive checksum does not match")
    files = {}
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        if sum(info.file_size for info in archive.infolist()) > config["max_unpacked_bytes"]:
            raise ValueError("upstream archive exceeds its unpacked ceiling")
        for info in archive.infolist():
            name = PurePosixPath(info.filename)
            if (
                name.is_absolute()
                or ".." in name.parts
                or "\\" in info.filename
                or ((info.external_attr >> 16) & 0o170000) == 0o120000
            ):
                raise ValueError("upstream archive contains an unsafe path")
            if info.is_dir():
                continue
            if info.filename in files:
                raise ValueError("upstream archive contains duplicate paths")
            files[info.filename] = archive.read(info)
    return files


def apply_changes(files, config):
    manifest_path = CONFIG / "patches.json"
    if not manifest_path.exists():
        raise ValueError("strong ingress patch set is not ready; no wheel was produced")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if manifest.get("upstream_sha256") != config["sha256"]:
        raise ValueError("patch set does not match the pinned upstream archive")
    changed = dict(files)
    for patch in manifest["files"]:
        path = patch["path"]
        if path not in files or digest(files[path]) != patch["before_sha256"]:
            raise ValueError("source checksum does not match the reviewed patch")
        text = files[path].decode("utf-8")
        for hunk in patch["hunks"]:
            if text.count(hunk["before"]) != 1:
                raise ValueError("patch context is missing or ambiguous")
            text = text.replace(hunk["before"], hunk["after"], 1)
        changed[path] = text.encode("utf-8")
        if digest(changed[path]) != patch["after_sha256"]:
            raise ValueError("patched source checksum does not match")
    for item in manifest["additions"]:
        target = PurePosixPath(item["path"])
        source = (CONFIG / item["source"]).resolve()
        if (
            target.is_absolute()
            or ".." in target.parts
            or not source.is_relative_to(CONFIG.resolve())
            or source.is_symlink()
            or item["path"] in changed
        ):
            raise ValueError("extension source path is unsafe or already present")
        data = source.read_bytes()
        if digest(data) != item["sha256"]:
            raise ValueError("extension source checksum does not match")
        changed[item["path"]] = data
    return changed


def write_wheel(files, config, output):
    old_info = f"telethon-{config['version']}.dist-info"
    new_info = f"telethon-{config['patched_version']}.dist-info"
    renamed = {}
    for name, data in files.items():
        if name.endswith(("/RECORD", "/RECORD.jws", "/RECORD.p7s")):
            continue
        target = name.replace(old_info + "/", new_info + "/", 1)
        if target == new_info + "/METADATA":
            text = data.decode("utf-8")
            before = f"Version: {config['version']}\n"
            if text.count(before) != 1:
                raise ValueError("upstream package version metadata is ambiguous")
            data = text.replace(before, f"Version: {config['patched_version']}\n", 1).encode(
                "utf-8"
            )
        renamed[target] = data
    receipt = {
        key: config[key]
        for key in ("version", "patched_version", "commit", "sha256", "extension_api")
    }
    receipt["module_hashes"] = {
        name: digest(data) for name, data in renamed.items() if name.endswith(".py")
    }
    renamed[new_info + "/kiaro-provenance.json"] = json.dumps(receipt, sort_keys=True).encode(
        "utf-8"
    )
    record = io.StringIO(newline="")
    writer = csv.writer(record, lineterminator="\n")
    for name, data in sorted(renamed.items()):
        hash_value = (
            base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode("ascii")
        )
        writer.writerow([name, "sha256=" + hash_value, len(data)])
    writer.writerow([new_info + "/RECORD", "", ""])
    renamed[new_info + "/RECORD"] = record.getvalue().encode("utf-8")
    output.mkdir(parents=True, exist_ok=True)
    target = output / f"telethon-{config['patched_version']}-py3-none-any.whl"
    if target.exists():
        raise ValueError("output wheel already exists; choose a fresh output directory")
    fd, temporary = tempfile.mkstemp(prefix=".telethon-wheel-", dir=output)
    try:
        with os.fdopen(fd, "wb") as stream:
            with zipfile.ZipFile(stream, "w", compression=zipfile.ZIP_DEFLATED) as archive:
                for name, data in sorted(renamed.items()):
                    archive.writestr(name, data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, target)
    finally:
        Path(temporary).unlink(missing_ok=True)
    return target


def init_logging():
    log = logging.getLogger("build_telethon")
    log.setLevel(logging.INFO)
    now = dt.datetime.now(dt.timezone.utc)
    path = ROOT / "logs" / f"build_telethon_{now:%Y-%m-%d_%H-%M-%S}_UTC.log"
    try:
        path.parent.mkdir(exist_ok=True)
        for index in range(100):
            candidate = path if index == 0 else path.with_stem(path.stem + f"_{index}")
            try:
                stream = candidate.open("x", encoding="utf-8")
                break
            except FileExistsError:
                continue
        else:
            raise OSError("log filename collision limit reached")
        handler = logging.StreamHandler(stream)
    except OSError:
        print("File logging unavailable; diagnostics use stderr.", file=sys.stderr)
        handler = logging.StreamHandler(sys.stderr)
    handler.setFormatter(
        logging.Formatter(
            "[%(asctime)s UTC] [%(levelname)s] [build_telethon] %(message)s",
            datefmt="%Y-%m-%d %H:%M:%S",
        )
    )
    handler.formatter.converter = __import__("time").gmtime
    log.addHandler(handler)
    return log, handler


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, help="Previously downloaded pinned upstream wheel")
    parser.add_argument("--output", type=Path, default=ROOT / "dist" / "prerequisites")
    args = parser.parse_args()
    log, handler = init_logging()
    try:
        config = json.loads((CONFIG / "upstream.json").read_text(encoding="utf-8"))
        log.info("Started pinned prerequisite build version=%s", config["version"])
        if args.archive:
            files = read_archive(args.archive, config)
        else:
            with tempfile.TemporaryDirectory(prefix="telethon-upstream-") as folder:
                path = Path(folder) / "upstream.whl"
                with urllib.request.urlopen(config["url"], timeout=30) as response:
                    data = response.read(config["max_download_bytes"] + 1)
                if len(data) > config["max_download_bytes"]:
                    raise ValueError("upstream archive exceeds its byte ceiling")
                path.write_bytes(data)
                files = read_archive(path, config)
        files = apply_changes(files, config)
        wheel = write_wheel(files, config, args.output)
        log.info("Completed patched wheel version=%s", config["patched_version"])
        print(wheel)
        return 0
    except (OSError, ValueError, KeyError, zipfile.BadZipFile) as failure:
        # Errors are ours or restricted metadata; never dump downloaded response bodies.
        message = (
            str(failure)
            if isinstance(failure, ValueError)
            else "prerequisite build failed; inspect the run log"
        )
        log.error("Failed type=%s", type(failure).__name__)
        print(message, file=sys.stderr)
        return 2
    finally:
        handler.flush()
        handler.close()
        if hasattr(handler, "stream") and handler.stream not in (sys.stdout, sys.stderr):
            handler.stream.close()
        log.removeHandler(handler)


if __name__ == "__main__":
    raise SystemExit(main())
