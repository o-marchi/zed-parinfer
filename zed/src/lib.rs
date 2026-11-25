use std::fs;
use zed_extension_api::{self as zed, LanguageServerId, LanguageServerInstallationStatus, Result};

const REPO: &str = "o-marchi/zed-parinfer";

struct Parinfer {
    downloaded: Option<String>,
}

impl zed::Extension for Parinfer {
    fn new() -> Self {
        Parinfer { downloaded: None }
    }

    fn language_server_command(
        &mut self,
        id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        // One on the PATH wins, so you can run your own build.
        let command = match worktree.which("parinfer-lsp") {
            Some(path) => path,
            None => self.download(id)?,
        };
        Ok(zed::Command {
            command,
            args: vec![],
            env: vec![],
        })
    }
}

impl Parinfer {
    // Fetches the latest release for this platform, once, and drops older ones.
    fn download(&mut self, id: &LanguageServerId) -> Result<String> {
        if let Some(path) = &self.downloaded {
            if fs::metadata(path).is_ok() {
                return Ok(path.clone());
            }
        }
        let status = |s| zed::set_language_server_installation_status(id, &s);
        status(LanguageServerInstallationStatus::CheckingForUpdate);
        let options = zed::GithubReleaseOptions {
            require_assets: true,
            pre_release: false,
        };
        let release = zed::latest_github_release(REPO, options)?;

        let (os, arch) = zed::current_platform();
        let arch = match arch {
            zed::Architecture::Aarch64 => "aarch64",
            zed::Architecture::X8664 => "x86_64",
            zed::Architecture::X86 => return Err("parinfer-lsp isn't built for 32-bit x86".into()),
        };
        let (target, archive, exe) = match os {
            zed::Os::Mac => ("apple-darwin", "tar.gz", ""),
            zed::Os::Linux => ("unknown-linux-musl", "tar.gz", ""),
            zed::Os::Windows => ("pc-windows-msvc", "zip", ".exe"),
        };
        let name = format!("parinfer-lsp-{arch}-{target}.{archive}");
        let asset = release
            .assets
            .iter()
            .find(|a| a.name == name)
            .ok_or_else(|| format!("release {} has no {name}", release.version))?;

        let dir = format!("parinfer-lsp-{}", release.version);
        let path = format!("{dir}/parinfer-lsp{exe}");
        if fs::metadata(&path).is_err() {
            status(LanguageServerInstallationStatus::Downloading);
            let kind = if exe.is_empty() {
                zed::DownloadedFileType::GzipTar
            } else {
                zed::DownloadedFileType::Zip
            };
            zed::download_file(&asset.download_url, &dir, kind)?;
            zed::make_file_executable(&path)?;
            for old in fs::read_dir(".").map_err(|e| e.to_string())?.flatten() {
                if old.file_name().to_str() != Some(dir.as_str()) {
                    fs::remove_dir_all(old.path()).ok();
                }
            }
        }
        status(LanguageServerInstallationStatus::None);
        self.downloaded = Some(path.clone());
        Ok(path)
    }
}

zed::register_extension!(Parinfer);
