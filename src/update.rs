use anyhow::Result;
use self_update::cargo_crate_version;

const GITHUB_OWNER: &str = "tzankich";
const GITHUB_REPO: &str = "timebox";

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub latest_version: String,
}

pub fn check_for_update() -> Result<Option<UpdateInfo>> {
    let current_version = cargo_crate_version!();

    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner(GITHUB_OWNER)
        .repo_name(GITHUB_REPO)
        .build()?
        .fetch()?;

    if let Some(latest) = releases.first() {
        let latest_version = latest.version.trim_start_matches('v').to_string();

        if latest_version != current_version {
            return Ok(Some(UpdateInfo {
                latest_version,
            }));
        }
    }

    Ok(None)
}

pub fn apply_update() -> Result<()> {
    let status = self_update::backends::github::Update::configure()
        .repo_owner(GITHUB_OWNER)
        .repo_name(GITHUB_REPO)
        .bin_name(get_bin_name())
        .target(get_target())
        .no_confirm(true)
        .current_version(cargo_crate_version!())
        .build()?
        .update()?;

    if status.updated() {
        println!("Updated to version {}", status.version());
    }

    Ok(())
}

fn get_bin_name() -> &'static str {
    #[cfg(target_os = "windows")]
    return "timebox.exe";

    #[cfg(not(target_os = "windows"))]
    return "timebox";
}

fn get_target() -> &'static str {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return "x86_64-pc-windows-msvc";

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return "x86_64-apple-darwin";

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return "aarch64-apple-darwin";

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return "x86_64-unknown-linux-gnu";

    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64"),
    )))]
    return "";
}
