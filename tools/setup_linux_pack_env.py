# -*- coding: utf-8 -*-
"""补齐 Windows 本机 Linux 打包环境，软件默认落在 D 盘。

步骤：
1. Zig → D:\\tools\\zig（供 cargo-zigbuild；完整 GUI 依赖时常仍不够）
2. Ubuntu 22.04 WSL → D:\\WSL\\StarQuakeLinux（与 CI 一致，推荐）
3. Rust 装在 WSL ext4（/root/.cargo），磁盘仍在 D:\\WSL\\*.vhdx

用法：
  python tools/setup_linux_pack_env.py          # Zig + cargo-zigbuild
  python tools/setup_linux_pack_env.py --wsl    # 导入/引导 WSL（需已重启启用组件）
  python tools/pack_linux.py                    # 优先 zigbuild，失败则走 WSL
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
import urllib.request
import zipfile
from pathlib import Path

ZIG_VERSION = "0.17.0"
DEST_ZIG = Path(os.environ.get("STARQUAKE_ZIG_HOME", r"D:\tools\zig"))
ZIG_URL = f"https://ziglang.org/download/{ZIG_VERSION}/zig-x86_64-windows-{ZIG_VERSION}.zip"
TMP_ZIP = Path(os.environ.get("TEMP", r"C:\Users\Huawei")) / f"_zig_{ZIG_VERSION}.zip"
TMP_DIR = Path(os.environ.get("TEMP", r"C:\Users\Huawei")) / f"_zig_{ZIG_VERSION}_extract"
WINGET_CANDIDATES = [
    Path(os.environ.get("LOCALAPPDATA", ""))
    / "Microsoft"
    / "WinGet"
    / "Packages"
    / "zig.zig_Microsoft.Winget.Source_8wekyb3d8bbwe"
    / f"zig-x86_64-windows-{ZIG_VERSION}",
]

DISTRO = os.environ.get("STARQUAKE_WSL_DISTRO", "StarQuakeLinux")
WSL_DIR = Path(os.environ.get("STARQUAKE_WSL_DIR", r"D:\WSL\StarQuakeLinux"))
ROOTFS = Path(os.environ.get("STARQUAKE_WSL_ROOTFS", r"D:\tools\ubuntu-22.04-wsl-rootfs.tar.gz"))
ROOTFS_URL = (
    "https://cloud-images.ubuntu.com/wsl/jammy/current/"
    "ubuntu-jammy-wsl-amd64-ubuntu22.04lts.rootfs.tar.gz"
)
RUSTUP_HOME = r"/root/.rustup"
CARGO_HOME = r"/root/.cargo"


def log(msg: str) -> None:
    print(msg, flush=True)


def add_user_path(entry: str) -> None:
    import winreg

    key = winreg.OpenKey(
        winreg.HKEY_CURRENT_USER, r"Environment", 0, winreg.KEY_READ | winreg.KEY_WRITE
    )
    try:
        cur, _ = winreg.QueryValueEx(key, "Path")
    except FileNotFoundError:
        cur = ""
    parts = [p for p in cur.split(";") if p]
    parts = [p for p in parts if p.lower() != entry.lower()]
    parts.insert(0, entry)
    winreg.SetValueEx(key, "Path", 0, winreg.REG_EXPAND_SZ, ";".join(parts))
    winreg.CloseKey(key)
    try:
        import ctypes

        ctypes.windll.user32.SendMessageW(0xFFFF, 0x001A, 0, "Environment")
    except Exception:
        pass
    log(f"USER PATH prepended: {entry}")


def ensure_rust_linux_target() -> None:
    r = subprocess.run(
        ["rustup", "target", "list", "--installed"],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if r.returncode != 0:
        raise SystemExit("rustup 不可用，请先安装 Rust 工具链")
    if "x86_64-unknown-linux-gnu" not in r.stdout:
        log("rustup target add x86_64-unknown-linux-gnu")
        subprocess.check_call(["rustup", "target", "add", "x86_64-unknown-linux-gnu"])
    else:
        log("rust target ok: x86_64-unknown-linux-gnu")


def ensure_cargo_zigbuild() -> None:
    r = subprocess.run(
        ["cargo", "zigbuild", "--help"],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if r.returncode == 0:
        log("cargo-zigbuild ok")
        return
    log("cargo install --locked cargo-zigbuild")
    subprocess.check_call(["cargo", "install", "--locked", "cargo-zigbuild"])


def install_zig() -> Path:
    DEST_ZIG.parent.mkdir(parents=True, exist_ok=True)
    zig_exe = DEST_ZIG / "zig.exe"
    if zig_exe.is_file():
        ver = subprocess.check_output([str(zig_exe), "version"], text=True).strip()
        log(f"Zig already at {DEST_ZIG} ({ver})")
        return DEST_ZIG

    src = next((p for p in WINGET_CANDIDATES if (p / "zig.exe").is_file()), None)
    if src is not None:
        log(f"copy Zig from WinGet: {src}")
        if DEST_ZIG.exists():
            shutil.rmtree(DEST_ZIG)
        shutil.copytree(src, DEST_ZIG)
    else:
        log(f"download {ZIG_URL}")
        urllib.request.urlretrieve(ZIG_URL, TMP_ZIP)
        if TMP_DIR.exists():
            shutil.rmtree(TMP_DIR)
        TMP_DIR.mkdir(parents=True)
        with zipfile.ZipFile(TMP_ZIP, "r") as zf:
            zf.extractall(TMP_DIR)
        found = next(TMP_DIR.rglob("zig.exe"))
        if DEST_ZIG.exists():
            shutil.rmtree(DEST_ZIG)
        shutil.copytree(found.parent, DEST_ZIG)
        TMP_ZIP.unlink(missing_ok=True)
        shutil.rmtree(TMP_DIR, ignore_errors=True)

    ver = subprocess.check_output([str(zig_exe), "version"], text=True).strip()
    log(f"installed {zig_exe} ({ver})")
    return DEST_ZIG


def wsl_distros() -> list[str]:
    r = subprocess.run(["wsl", "-l", "-q"], capture_output=True)
    if r.returncode != 0:
        return []
    for enc in ("utf-16-le", "utf-8"):
        try:
            text = r.stdout.decode(enc)
            names = [ln.replace("\x00", "").strip() for ln in text.splitlines()]
            names = [n for n in names if n and "wsl.exe" not in n.lower()]
            if names:
                return names
        except Exception:
            continue
    return []


def ensure_rootfs() -> None:
    if ROOTFS.is_file() and ROOTFS.stat().st_size > 50_000_000:
        log(f"rootfs ok: {ROOTFS}")
        return
    ROOTFS.parent.mkdir(parents=True, exist_ok=True)
    log(f"download {ROOTFS_URL}")
    urllib.request.urlretrieve(ROOTFS_URL, ROOTFS)
    log(f"saved {ROOTFS} ({ROOTFS.stat().st_size} bytes)")


def import_wsl() -> None:
    names = wsl_distros()
    if any(n.lower() == DISTRO.lower() for n in names):
        log(f"WSL distro exists: {DISTRO}")
        return
    ensure_rootfs()
    WSL_DIR.parent.mkdir(parents=True, exist_ok=True)
    if WSL_DIR.exists() and any(WSL_DIR.iterdir()):
        raise SystemExit(f"目录非空，请清空或改 STARQUAKE_WSL_DIR: {WSL_DIR}")
    cmd = [
        "wsl",
        "--import",
        DISTRO,
        str(WSL_DIR),
        str(ROOTFS),
        "--version",
        "2",
    ]
    log(" ".join(cmd))
    r = subprocess.run(cmd)
    if r.returncode != 0:
        raise SystemExit(
            "WSL import 失败。若提示可选组件未就绪，请重启后再执行："
            " python tools/setup_linux_pack_env.py --wsl"
        )


def bootstrap_wsl() -> None:
    # apt deps + rust on D:
    script = f"""
set -e
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y build-essential pkg-config curl ca-certificates \\
  libasound2-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \\
  libxkbcommon-dev libssl-dev libgtk-3-dev
export RUSTUP_HOME={RUSTUP_HOME}
export CARGO_HOME={CARGO_HOME}
if [ ! -x "$CARGO_HOME/bin/cargo" ]; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
fi
# profile for root (imported distros login as root)
grep -q STARQUAKE_RUST_ENV /root/.bashrc 2>/dev/null || cat >> /root/.bashrc <<'EOF'

# STARQUAKE_RUST_ENV
export RUSTUP_HOME={RUSTUP_HOME}
export CARGO_HOME={CARGO_HOME}
export PATH="$CARGO_HOME/bin:$PATH"
EOF
. "$CARGO_HOME/env"
rustc -V
cargo -V
uname -a
"""
    cmd = ["wsl", "-d", DISTRO, "-u", "root", "--", "bash", "-lc", script]
    log(f"bootstrap {DISTRO} (apt + rust in WSL ext4) ...")
    r = subprocess.run(cmd)
    if r.returncode != 0:
        raise SystemExit("WSL bootstrap 失败")
    log("WSL bootstrap ok")


def setup_zig() -> None:
    zig_home = install_zig()
    add_user_path(str(zig_home))
    os.environ["PATH"] = str(zig_home) + os.pathsep + os.environ.get("PATH", "")
    ensure_rust_linux_target()
    ensure_cargo_zigbuild()
    log(f"Zig: {shutil.which('zig')}")


def setup_wsl() -> None:
    # Detect stub / optional component
    st = subprocess.run(["wsl", "--status"], capture_output=True)
    err = (st.stderr or b"").decode("utf-8", "replace") + (st.stdout or b"").decode(
        "utf-8", "replace"
    )
    if "OPTIONAL_COMPONENT" in err or st.returncode != 0:
        # try list
        if not wsl_distros() and "import" not in err.lower():
            probe = subprocess.run(["wsl", "--import", "/?"], capture_output=True)
            # If still stub help only:
            helpb = subprocess.run(["wsl", "--help"], capture_output=True)
            ht = helpb.stdout.decode("utf-8", "replace")
            if "--import" not in ht and "import" not in ht.lower():
                raise SystemExit(
                    "WSL 可选组件尚未生效，请先重启 Windows，再运行：\n"
                    "  python tools/setup_linux_pack_env.py --wsl"
                )
    import_wsl()
    bootstrap_wsl()
    log("")
    log(f"WSL 就绪：wsl -d {DISTRO}")
    log("打包：python tools/pack_linux.py")


def main() -> int:
    if sys.platform != "win32":
        log("非 Windows：直接 cargo build -p starquake-app --release && python tools/pack_linux.py --skip-build")
        return 0
    args = set(sys.argv[1:])
    do_wsl = "--wsl" in args or "--all" in args
    do_zig = "--zig" in args or "--all" in args or not do_wsl
    # default: zig only; --wsl for wsl; --all for both
    if "--all" in args:
        do_zig = True
        do_wsl = True
    if do_zig:
        setup_zig()
    if do_wsl:
        setup_wsl()
    if not do_wsl:
        log("")
        log("提示：完整 Linux GUI 依赖建议再执行：")
        log("  python tools/setup_linux_pack_env.py --wsl")
        log("（需已启用 WSL 可选组件；若刚装过功能请先重启）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
