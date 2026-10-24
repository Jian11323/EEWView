//! 从静态更新清单检查 / 下载 / 应用更新。
//!
//! 并行拉取 GitHub 与 Gitee 上的 `update/manifest.json` 与更新包，先成功者优先。

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use tracing::{info, warn};

const UA: &str = concat!(
    "StarQuake/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/Jian11323/StarQuake; +https://gitee.com/jian0786/starquake)"
);

/// 清单地址（GitHub / Gitee 并行竞速，先成功者优先）。
const MANIFEST_URLS: &[&str] = &[
    "https://gitee.com/jian0786/starquake/raw/master/update/manifest.json",
    "https://gitee.com/jian0786/starquake/raw/main/update/manifest.json",
    "https://raw.githubusercontent.com/Jian11323/StarQuake/main/update/manifest.json",
    "https://raw.githubusercontent.com/Jian11323/StarQuake/master/update/manifest.json",
];

const GITEE_REPO_PREFIX: &str = "https://gitee.com/jian0786/starquake/";
const GITHUB_REPO_PREFIX: &str = "https://github.com/Jian11323/StarQuake/";

#[derive(Debug, Clone)]
pub struct AvailableUpdate {
    pub version: String,
    pub name: String,
    pub body: String,
    pub html_url: String,
    pub asset_url: String,
    pub asset_name: String,
    /// 空则跳过校验
    pub sha256: String,
}

#[derive(Debug, Deserialize)]
struct UpdateManifest {
    latest_version: String,
    #[serde(default)]
    min_supported_version: String,
    #[serde(default)]
    channel: String,
    #[serde(default)]
    #[cfg_attr(not(windows), allow(dead_code))]
    installer: Option<ManifestAsset>,
    #[serde(default)]
    #[cfg_attr(not(windows), allow(dead_code))]
    zip: Option<ManifestAsset>,
    #[serde(default)]
    #[cfg_attr(windows, allow(dead_code))]
    linux: Option<ManifestAsset>,
    /// 更新日志正文（弹窗展示）
    #[serde(default)]
    notes: String,
    #[serde(default)]
    release_notes_url: String,
    #[serde(default)]
    html_url: String,
}

#[derive(Debug, Deserialize, Clone)]
struct ManifestAsset {
    url: String,
    #[serde(default)]
    sha256: String,
}

/// 解析 `v2.0.1` / `2.0.1` / `StarQuake-2.0.1` → (major, minor, patch)。
pub fn parse_semver(raw: &str) -> Option<(u64, u64, u64)> {
    let s = raw.trim().trim_start_matches('v').trim_start_matches('V');
    let s = s
        .rsplit([' ', '-', '_'])
        .find(|p| p.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .unwrap_or(s);
    let mut parts = s.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts
        .next()
        .unwrap_or("0")
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()?;
    Some((major, minor, patch))
}

pub fn is_newer(remote: &str, local: &str) -> bool {
    match (parse_semver(remote), parse_semver(local)) {
        (Some(r), Some(l)) => r > l,
        _ => false,
    }
}

fn http_client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(UA)
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::limited(8))
        .build()?)
}

fn try_fetch_manifest_one(
    client: &reqwest::blocking::Client,
    url: &str,
) -> Result<UpdateManifest> {
    let resp = client.get(url).send().with_context(|| format!("请求 {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        let snippet: String = body.chars().take(160).collect();
        bail!("HTTP {status} {snippet}");
    }
    // 兼容带 BOM 的 UTF-8
    let bytes = resp.bytes().context("读取清单正文")?;
    let text = String::from_utf8_lossy(&bytes);
    let text = text.trim_start_matches('\u{feff}');
    serde_json::from_str::<UpdateManifest>(text).context("解析 JSON")
}

fn fetch_manifest(client: &reqwest::blocking::Client) -> Result<UpdateManifest> {
    let (tx, rx) = mpsc::channel();
    let n = MANIFEST_URLS.len();
    for url in MANIFEST_URLS {
        let tx = tx.clone();
        let client = client.clone();
        let url = (*url).to_string();
        std::thread::spawn(move || {
            let result = try_fetch_manifest_one(&client, &url);
            let _ = tx.send((url, result));
        });
    }
    drop(tx);

    let mut errors = Vec::new();
    let mut remaining = n;
    while remaining > 0 {
        match rx.recv() {
            Ok((url, Ok(m))) => {
                info!(url = %url, ver = %m.latest_version, "更新清单已加载");
                return Ok(m);
            }
            Ok((url, Err(e))) => {
                errors.push(format!("{url} → {e:#}"));
                remaining -= 1;
            }
            Err(_) => break,
        }
    }
    bail!(
        "无法获取更新清单：{}",
        if errors.is_empty() {
            "无可用镜像".into()
        } else {
            errors.join("；")
        }
    )
}

/// 安装到 Program Files 时优先用 setup.exe（需管理员，zip 直拷会失败）。
#[cfg(windows)]
fn running_from_program_files() -> bool {
    std::env::current_exe()
        .ok()
        .map(|p| {
            let s = p.to_string_lossy().to_ascii_lowercase();
            s.contains("program files")
        })
        .unwrap_or(false)
}

fn pick_platform_asset(m: &UpdateManifest) -> Result<(ManifestAsset, String)> {
    #[cfg(windows)]
    {
        let zip = m.zip.as_ref().filter(|a| !a.url.trim().is_empty());
        let installer = m.installer.as_ref().filter(|a| !a.url.trim().is_empty());
        // 已安装版 → setup 优先；便携版 → zip 优先
        let order: [Option<&ManifestAsset>; 2] = if running_from_program_files() {
            [installer, zip]
        } else {
            [zip, installer]
        };
        for a in order.into_iter().flatten() {
            let name = url_file_name(&a.url).unwrap_or_else(|| {
                if a.url.to_ascii_lowercase().contains(".exe") {
                    "StarQuake_setup.exe".into()
                } else {
                    "StarQuake_portable.zip".into()
                }
            });
            return Ok((a.clone(), name));
        }
        bail!("更新清单缺少 Windows 安装包 / 便携包 URL");
    }
    #[cfg(not(windows))]
    {
        if let Some(l) = m.linux.as_ref().filter(|a| !a.url.trim().is_empty()) {
            let name = url_file_name(&l.url).unwrap_or_else(|| "StarQuake_linux.tar.gz".into());
            return Ok((l.clone(), name));
        }
        bail!("更新清单缺少 Linux 包 URL");
    }
}

fn url_file_name(url: &str) -> Option<String> {
    let path = url.split('?').next().unwrap_or(url);
    path.rsplit('/')
        .next()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

/// 由清单里的主 URL 推导 GitHub / Gitee 镜像，去重后并行下载。
fn asset_mirror_urls(primary: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut push = |u: String| {
        if !u.trim().is_empty() && !out.iter().any(|x| x == &u) {
            out.push(u);
        }
    };
    push(primary.trim().to_string());
    if let Some(rest) = primary.strip_prefix(GITEE_REPO_PREFIX) {
        push(format!("{GITHUB_REPO_PREFIX}{rest}"));
    }
    if let Some(rest) = primary.strip_prefix(GITHUB_REPO_PREFIX) {
        push(format!("{GITEE_REPO_PREFIX}{rest}"));
    }
    out
}

/// 查询最新版；若无更新返回 `Ok(None)`。
pub fn check_latest(local_version: &str) -> Result<Option<AvailableUpdate>> {
    let client = http_client()?;
    let m = fetch_manifest(&client)?;
    let ver = m
        .latest_version
        .trim()
        .trim_start_matches('v')
        .trim_start_matches('V')
        .to_string();
    if ver.is_empty() {
        bail!("更新清单缺少 latest_version");
    }
    if !is_newer(&ver, local_version) {
        info!(
            local = local_version,
            remote = %ver,
            channel = %m.channel,
            "已是最新版"
        );
        let _ = m.min_supported_version;
        return Ok(None);
    }
    let (asset, asset_name) = pick_platform_asset(&m)?;
    let html = if !m.html_url.trim().is_empty() {
        m.html_url
    } else if !m.release_notes_url.trim().is_empty() {
        m.release_notes_url.clone()
    } else {
        format!("https://gitee.com/jian0786/starquake/releases/tag/v{ver}")
    };
    let body = if !m.notes.trim().is_empty() {
        m.notes.trim().to_string()
    } else if !m.release_notes_url.trim().is_empty() {
        format!("发行说明：{}", m.release_notes_url.trim())
    } else {
        String::new()
    };
    Ok(Some(AvailableUpdate {
        version: ver.clone(),
        name: format!("StarQuake {ver}"),
        body,
        html_url: html,
        asset_url: asset.url,
        asset_name,
        sha256: asset.sha256.trim().to_ascii_lowercase(),
    }))
}

/// 下载前重置暂存目录（应用阶段禁止再调用，以免删掉已解压内容）。
fn reset_staging() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("StarQuake_update");
    if dir.exists() {
        let _ = fs::remove_dir_all(&dir);
    }
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn sha256_hex(path: &Path) -> Result<String> {
    // 进程内计算，避免 certutil 输出带空格/本地化文案导致解析失败。
    let mut file = File::open(path).with_context(|| format!("打开 {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).context("读取更新包")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn download_one_url(
    client: &reqwest::blocking::Client,
    url: &str,
    dest: &Path,
    expect_sha: &str,
) -> Result<()> {
    info!(url = %url, dest = %dest.display(), "下载更新包");
    let mut resp = client.get(url).send().context("下载更新包")?;
    if !resp.status().is_success() {
        bail!("HTTP {}", resp.status());
    }
    let mut file = File::create(dest)?;
    io::copy(&mut resp, &mut file)?;
    file.flush()?;
    if !expect_sha.is_empty() {
        let got = sha256_hex(dest)?;
        if got != expect_sha {
            let _ = fs::remove_file(dest);
            bail!("校验失败（期望 {expect_sha}，实际 {got}）");
        }
        info!(url = %url, "更新包 SHA256 校验通过");
    }
    Ok(())
}

/// 下载资源到暂存目录，返回本地文件路径。
/// GitHub / Gitee 镜像并行下载，先成功（且校验通过）者优先。
pub fn download_asset(info: &AvailableUpdate) -> Result<PathBuf> {
    let dir = reset_staging()?;
    let dest = dir.join(&info.asset_name);
    let urls = asset_mirror_urls(&info.asset_url);
    if urls.is_empty() {
        bail!("更新包 URL 为空");
    }

    let client = reqwest::blocking::Client::builder()
        .user_agent(UA)
        .timeout(std::time::Duration::from_secs(600))
        .redirect(reqwest::redirect::Policy::limited(8))
        .build()?;

    let (tx, rx) = mpsc::channel();
    let n = urls.len();
    for (i, url) in urls.into_iter().enumerate() {
        let tx = tx.clone();
        let client = client.clone();
        let part = dir.join(format!("part_{i}_{}", info.asset_name));
        let sha = info.sha256.clone();
        std::thread::spawn(move || {
            let result = download_one_url(&client, &url, &part, &sha).map(|_| (url, part));
            let _ = tx.send(result);
        });
    }
    drop(tx);

    let mut errors = Vec::new();
    let mut remaining = n;
    while remaining > 0 {
        match rx.recv() {
            Ok(Ok((url, part))) => {
                if part != dest {
                    fs::rename(&part, &dest)
                        .or_else(|_| {
                            fs::copy(&part, &dest)?;
                            fs::remove_file(&part)?;
                            Ok::<(), io::Error>(())
                        })
                        .with_context(|| format!("落盘 {}", dest.display()))?;
                }
                info!(url = %url, dest = %dest.display(), "更新包下载完成");
                return Ok(dest);
            }
            Ok(Err(e)) => {
                errors.push(format!("{e:#}"));
                remaining -= 1;
            }
            Err(_) => break,
        }
    }
    bail!(
        "下载失败：{}",
        if errors.is_empty() {
            "无可用镜像".into()
        } else {
            errors.join("；")
        }
    )
}

/// 应用已下载的更新包并重启（Windows 便携 zip / setup.exe；Linux tar.gz）。
pub fn apply_and_relaunch(package: &Path) -> Result<()> {
    let exe = std::env::current_exe().context("current_exe")?;
    let app_dir = exe
        .parent()
        .map(Path::to_path_buf)
        .context("exe parent")?;
    let name = package
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    #[cfg(windows)]
    if name.ends_with(".exe") {
        // 勿再调用会清空暂存的逻辑；bat 与 setup 同目录，先退出本进程再静默安装
        let stage = package
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| std::env::temp_dir().join("StarQuake_update"));
        let bat = stage.join("apply_installer.cmd");
        let setup = package.display().to_string();
        let body = format!(
            "@echo off\r\n\
chcp 65001 >nul\r\n\
title StarQuake update\r\n\
echo.\r\n\
echo   Updating StarQuake, please wait...\r\n\
echo.\r\n\
timeout /t 2 /nobreak >nul\r\n\
if not exist \"{setup}\" goto :eof\r\n\
\"{setup}\" /S\r\n\
set \"L1=%ProgramFiles(x86)%\\StarQuake\\starquake.exe\"\r\n\
set \"L2=%ProgramFiles%\\StarQuake\\starquake.exe\"\r\n\
set \"L3=%LocalAppData%\\Programs\\StarQuake\\starquake.exe\"\r\n\
if exist \"%L1%\" (start \"\" \"%L1%\" & goto :done)\r\n\
if exist \"%L2%\" (start \"\" \"%L2%\" & goto :done)\r\n\
if exist \"%L3%\" (start \"\" \"%L3%\" & goto :done)\r\n\
:done\r\n\
del \"%~f0\"\r\n",
            setup = setup,
        );
        fs::write(&bat, body)?;
        Command::new("cmd")
            .args(["/C", &bat.to_string_lossy()])
            .spawn()
            .context("启动安装脚本")?;
        std::process::exit(0);
    }

    let extract_dir = package
        .parent()
        .unwrap_or(Path::new("."))
        .join("extracted");
    if extract_dir.exists() {
        let _ = fs::remove_dir_all(&extract_dir);
    }
    fs::create_dir_all(&extract_dir)?;

    if name.ends_with(".zip") {
        extract_zip(package, &extract_dir)?;
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        extract_tar_gz(package, &extract_dir)?;
    } else {
        bail!("不支持的更新包格式: {name}");
    }

    let payload = find_payload_root(&extract_dir)?;
    #[cfg(windows)]
    {
        // 必须与 package 同级写 bat，禁止 reset 暂存（否则会删掉刚解压的 payload）
        let bat = package
            .parent()
            .unwrap_or(Path::new("."))
            .join("apply_update.cmd");
        let exe_name = exe
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("starquake.exe");
        let body = format!(
            "@echo off\r\n\
chcp 65001 >nul\r\n\
title StarQuake update\r\n\
echo.\r\n\
echo   Updating StarQuake, please wait...\r\n\
echo.\r\n\
timeout /t 2 /nobreak >nul\r\n\
xcopy /E /Y /I /Q \"{src}\\*\" \"{dst}\\\"\r\n\
if exist \"{dst}\\{exe}\" start \"\" \"{dst}\\{exe}\"\r\n\
del \"%~f0\"\r\n",
            src = payload.display(),
            dst = app_dir.display(),
            exe = exe_name,
        );
        fs::write(&bat, body)?;
        Command::new("cmd")
            .args(["/C", &bat.to_string_lossy()])
            .spawn()
            .context("启动更新脚本")?;
        std::process::exit(0);
    }
    #[cfg(not(windows))]
    {
        let _ = package;
        copy_dir_recursive(&payload, &app_dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let bin = app_dir.join("starquake");
            if bin.is_file() {
                let mut perms = fs::metadata(&bin)?.permissions();
                perms.set_mode(0o755);
                let _ = fs::set_permissions(&bin, perms);
            }
        }
        Command::new(&exe).spawn().context("重启应用")?;
        std::process::exit(0);
    }
}

fn find_payload_root(extract_dir: &Path) -> Result<PathBuf> {
    for name in ["StarQuake", "EEWView", "starquake", "eewview"] {
        let p = extract_dir.join(name);
        if p.is_dir() {
            return Ok(p);
        }
    }
    let mut dirs = fs::read_dir(extract_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect::<Vec<_>>();
    if dirs.len() == 1 {
        return Ok(dirs.remove(0));
    }
    Ok(extract_dir.to_path_buf())
}

fn extract_zip(zip_path: &Path, dest: &Path) -> Result<()> {
    let file = File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file).context("打开 zip")?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let out = match entry.enclosed_name() {
            Some(p) => dest.join(p),
            None => continue,
        };
        if entry.name().ends_with('/') {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut outfile = File::create(&out)?;
        io::copy(&mut entry, &mut outfile)?;
    }
    Ok(())
}

fn extract_tar_gz(path: &Path, dest: &Path) -> Result<()> {
    let status = Command::new("tar")
        .args([
            "-xzf",
            &path.to_string_lossy(),
            "-C",
            &dest.to_string_lossy(),
        ])
        .status()
        .context("执行 tar")?;
    if !status.success() {
        bail!("tar 解压失败");
    }
    Ok(())
}

#[cfg(not(windows))]
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for ent in fs::read_dir(src)? {
        let ent = ent?;
        let from = ent.path();
        let to = dst.join(ent.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

pub fn open_release_page(url: &str) {
    #[cfg(windows)]
    {
        if let Err(e) = Command::new("cmd").args(["/C", "start", "", url]).spawn() {
            warn!("无法打开发布页: {e}");
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Err(e) = Command::new("open").arg(url).spawn() {
            warn!("无法打开发布页: {e}");
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Err(e) = Command::new("xdg-open").arg(url).spawn() {
            warn!("无法打开发布页: {e}");
        }
    }
}

/// 启动主窗口前的检查结果（安装成功时通常已 `process::exit`，不会回到调用方）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupUpdateOutcome {
    /// 继续进主界面；`checked=true` 表示本轮已查过，主界面内勿再自动查
    Continue { checked: bool },
    /// 已触发安装并应退出（兜底；正常路径在 `apply_and_relaunch` 内 exit）
    Exit,
}

/// 启动主窗口前检查更新：复用 `check_latest` / `download_asset` / `apply_and_relaunch`。
///
/// 流程对齐「滚动字幕」：有新版本则原生对话框询问；确认后下载安装并退出，不创建主窗口。
/// `cfg.last_dismissed_update_version` 可能被改写；调用方应在 Continue 时按需 `save_user_overlay`。
pub fn run_startup_auto_update(
    cfg: &mut starquake_config::UpdateConfig,
    local_version: &str,
) -> StartupUpdateOutcome {
    if !cfg.check_on_startup {
        return StartupUpdateOutcome::Continue { checked: false };
    }

    // 非 Windows 无 MessageBox 确认：留给主界面内弹窗
    #[cfg(not(windows))]
    {
        let _ = local_version;
        return StartupUpdateOutcome::Continue { checked: false };
    }

    #[cfg(windows)]
    {
        let avail = match check_latest(local_version) {
            Ok(Some(info)) => info,
            Ok(None) => return StartupUpdateOutcome::Continue { checked: true },
            Err(e) => {
                warn!("启动前检查更新失败（已跳过）: {e:#}");
                return StartupUpdateOutcome::Continue { checked: true };
            }
        };

        let dismissed = cfg.last_dismissed_update_version.trim();
        if !dismissed.is_empty() && dismissed == avail.version.trim() {
            info!(
                ver = %avail.version,
                "启动前检查更新：已跳过该版本，不再询问"
            );
            return StartupUpdateOutcome::Continue { checked: true };
        }

        let body = avail.body.trim();
        let ask = if body.is_empty() {
            format!(
                "发现新版本 v{}（当前 v{}）。\n是否下载并安装？\n\n安装完成后将自动启动新版本。",
                avail.version, local_version
            )
        } else {
            let notes = if body.chars().count() > 400 {
                let t: String = body.chars().take(400).collect();
                format!("{t}…")
            } else {
                body.to_string()
            };
            format!(
                "发现新版本 v{}（当前 v{}）。\n是否下载并安装？\n\n安装完成后将自动启动新版本。\n\n{}",
                avail.version, local_version, notes
            )
        };

        if !native_yes_no("发现新版本", &ask) {
            cfg.last_dismissed_update_version = avail.version.clone();
            info!(ver = %avail.version, "启动前检查更新：用户暂不更新");
            return StartupUpdateOutcome::Continue { checked: true };
        }

        if !cfg.auto_install {
            open_release_page(&avail.html_url);
            return StartupUpdateOutcome::Continue { checked: true };
        }

        info!(ver = %avail.version, "启动前检查更新：开始下载");
        match download_asset(&avail) {
            Ok(path) => {
                if let Err(e) = apply_and_relaunch(&path) {
                    warn!("启动前应用更新失败: {e:#}");
                    native_info(
                        "更新失败",
                        &format!("应用更新失败：{e:#}\n将继续启动当前版本。"),
                    );
                    StartupUpdateOutcome::Continue { checked: true }
                } else {
                    StartupUpdateOutcome::Exit
                }
            }
            Err(e) => {
                warn!("启动前下载更新失败: {e:#}");
                native_info(
                    "更新失败",
                    &format!("下载失败：{e:#}\n将继续启动当前版本。"),
                );
                StartupUpdateOutcome::Continue { checked: true }
            }
        }
    }
}

#[cfg(windows)]
fn native_yes_no(title: &str, message: &str) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONQUESTION, MB_YESNO, IDYES,
    };
    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }
    let text = wide(message);
    let caption = wide(title);
    let r = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_YESNO | MB_ICONQUESTION,
        )
    };
    r == IDYES
}

#[cfg(windows)]
fn native_info(title: &str, message: &str) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONINFORMATION, MB_OK,
    };
    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }
    let text = wide(message);
    let caption = wide(title);
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_newer() {
        assert!(is_newer("2.0.1", "2.0.0"));
        assert!(is_newer("v2.1.0", "2.0.9"));
        assert!(!is_newer("2.0.0", "2.0.0"));
        assert!(!is_newer("1.9.9", "2.0.0"));
    }

    #[test]
    fn parse_manifest_shape() {
        let raw = r#"{
          "latest_version": "2.0.1",
          "notes": "hello",
          "zip": { "url": "https://example.com/a.zip", "sha256": "" },
          "installer": { "url": "https://example.com/a.exe", "sha256": "" }
        }"#;
        let m: UpdateManifest = serde_json::from_str(raw).unwrap();
        assert_eq!(m.latest_version, "2.0.1");
        assert_eq!(m.notes, "hello");
        assert!(m.zip.unwrap().url.contains("a.zip"));
    }

    #[test]
    fn mirror_urls_gitee_and_github() {
        let gitee = "https://gitee.com/jian0786/starquake/releases/download/v2.2.2/a.zip";
        let urls = asset_mirror_urls(gitee);
        assert_eq!(urls.len(), 2);
        assert!(urls.iter().any(|u| u.contains("github.com/Jian11323")));
        assert!(urls.iter().any(|u| u.contains("gitee.com/jian0786")));

        let github = "https://github.com/Jian11323/StarQuake/releases/download/v2.2.2/a.zip";
        let urls2 = asset_mirror_urls(github);
        assert_eq!(urls2.len(), 2);
    }

    #[test]
    fn reset_staging_then_keep_payload_paths() {
        // 回归：apply 阶段不得再 wipe 暂存，否则 bat 指向的解压目录会消失
        let dir = reset_staging().unwrap();
        let marker = dir.join("keep_me.txt");
        fs::write(&marker, b"ok").unwrap();
        assert!(marker.is_file());
        // 二次 create_dir_all 式「仅确保存在」不得删文件
        fs::create_dir_all(&dir).unwrap();
        assert!(marker.is_file(), "apply 写 bat 时不得清空暂存");
        let _ = fs::remove_dir_all(&dir);
    }
}
