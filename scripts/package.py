"""Packs a release of fterm: a portable folder in an archive (no install).

    python scripts/package.py --bin-dir target/release --out dist

Windows gives fterm-<version>-windows-<arch>.zip, Linux and macOS a .tar.gz. In the archive is one folder
fterm-<version>/ with the programs, a portable fterm.lua (data_dir = "data"), the README, and the licenses.
A .sha256 file comes next to the archive.
"""

import argparse
import hashlib
import platform
import re
import shutil
import sys
import tarfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def version() -> str:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'^\[workspace\.package\][^\[]*?^version\s*=\s*"([^"]+)"', text, re.M | re.S)
    if not match:
        sys.exit("no version in [workspace.package] of Cargo.toml")
    return match.group(1)


def os_name() -> str:
    return {"Windows": "windows", "Linux": "linux", "Darwin": "macos"}[platform.system()]


def arch_name() -> str:
    machine = platform.machine().lower()
    return {"amd64": "x64", "x86_64": "x64", "arm64": "arm64", "aarch64": "arm64"}.get(machine, machine)


def files(bin_dir: Path, system: str) -> list[tuple[Path, str]]:
    """(source, name in the archive)."""
    exe = ".exe" if system == "windows" else ""
    out = [(bin_dir / f"fterm{exe}", f"fterm{exe}"), (bin_dir / f"ftermctl{exe}", f"ftermctl{exe}")]
    out += [
        (ROOT / "assets/portable/fterm.lua", "fterm.lua"),
        (ROOT / "assets/portable/README.txt", "README.txt"),
        (ROOT / "LICENSE", "LICENSE"),
        (ROOT / "THIRD-PARTY-NOTICES.md", "THIRD-PARTY-NOTICES.md"),
        (ROOT / "assets/fonts/OFL.txt", "OFL.txt"),
    ]
    if system == "linux":
        out += [(ROOT / "assets/linux/fterm.desktop", "fterm.desktop"), (ROOT / "assets/icon/fterm-256.png", "fterm.png")]
    missing = [str(src) for src, _ in out if not src.is_file()]
    if missing:
        sys.exit("missing: " + ", ".join(missing))
    return out


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", default="target/release", help="where fterm and ftermctl are")
    parser.add_argument("--out", default="dist")
    parser.add_argument("--tag", help="the git tag (v1.2.3): it must be the version of Cargo.toml")
    args = parser.parse_args()

    ver = version()
    if args.tag and args.tag != f"v{ver}":
        sys.exit(f"the tag {args.tag} is not the version of Cargo.toml (v{ver})")
    system, arch = os_name(), arch_name()
    name = f"fterm-{ver}-{system}-{arch}"
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    folder = f"fterm-{ver}"
    items = files(Path(args.bin_dir), system)

    if system == "windows":
        archive = out / f"{name}.zip"
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as z:
            for src, inside in items:
                z.write(src, f"{folder}/{inside}")
    else:
        archive = out / f"{name}.tar.gz"
        with tarfile.open(archive, "w:gz") as t:
            for src, inside in items:
                info = t.gettarinfo(str(src), f"{folder}/{inside}")
                # The programs can run; the other files are plain files.
                info.mode = 0o755 if inside in ("fterm", "ftermctl") else 0o644
                with open(src, "rb") as f:
                    t.addfile(info, f)

    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    (out / f"{archive.name}.sha256").write_text(f"{digest}  {archive.name}\n", encoding="utf-8")
    print(archive)


if __name__ == "__main__":
    main()
