# Linux 打包（本机 Windows）

与 CI（`portable-linux.yml` / Ubuntu 22.04）对齐：优先在 WSL 里原生 `cargo build`，再 `python tools/pack_linux.py`。

## 软件位置（默认 D 盘）

| 组件 | 路径 |
|------|------|
| Zig | `D:\tools\zig` |
| Ubuntu 22.04 WSL | `D:\WSL\StarQuakeLinux` |
| WSL 内 Rust | `/root/.rustup`、`/root/.cargo`（落在 `D:\WSL\StarQuakeLinux\ext4.vhdx`） |
| rootfs 缓存 | `D:\tools\ubuntu-22.04-wsl-rootfs.tar.gz` |

## 一次性准备

```bat
python tools/setup_linux_pack_env.py
```

若刚启用过「适用于 Linux 的 Windows 子系统 / 虚拟机平台」，**先重启**，再：

```bat
python tools/setup_linux_pack_env.py --wsl
```

（`--all` = Zig + WSL 一并做。）

## 打包

```bat
python tools/pack_linux.py
```

脚本顺序：Windows 上先 `cargo zigbuild`；失败则自动 `wsl -d StarQuakeLinux` 原生编译并打 `dist/StarQuake_<ver>_linux_x86_64.tar.gz`。

## 说明

- 纯 Zig 交叉缺少 alsa/gtk 的 sysroot，完整打包请用 WSL。
- 环境变量可覆盖路径：`STARQUAKE_ZIG_HOME`、`STARQUAKE_WSL_DISTRO`、`STARQUAKE_WSL_DIR`、`STARQUAKE_WSL_ROOTFS`。
