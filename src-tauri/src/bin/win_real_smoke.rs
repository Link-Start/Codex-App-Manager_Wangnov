use std::path::{Path, PathBuf};
use std::process::ExitCode;

use codex_app_manager_lib::adapters::host;
use codex_app_manager_lib::app::provenance::ProvenanceStore;
use codex_app_manager_lib::app::win_update::{
    perform_windows_update, uninstall_windows_codex, win_adopt, win_install_status,
    WinInstallStatus,
};
use codex_app_manager_lib::domain::manifest::MirrorEndpoints;
use codex_app_manager_lib::domain::settings::AppSettings;
use codex_app_manager_lib::domain::target::Target;
use codex_win_engine::{
    download_to, fetch_text, find_msix_sha256, install_msix_sideload, install_portable_from_msix,
    parse_manifest, read_msix_identity, sha256_file, validate_codex_identity,
    verify_openai_authenticode, version_key, WindowsRelease,
};
use serde::Serialize;

const OLD_MSIX_VERSION: &str = "26.601.2237.0";
const OLD_MSIX_MONIKER: &str = "OpenAI.Codex_26.601.2237.0_x64__2p2nqsd0c76g0";
const OLD_MSIX_URL: &str = "https://github.com/Wangnov/codex-app-mirror/releases/download/codex-app-win-26.601.2237.0-mac-26.601.21317-b3511/OpenAI.Codex_26.601.2237.0_x64__2p2nqsd0c76g0.Msix";
const OLD_MSIX_SHA256: &str = "432d5e75ee973bf8172db58435a86823ccd51272ea0a82395c70a6d485015caa";
const OLD_MSIX_SIZE: u64 = 564_451_929;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StepReport {
    step: String,
    ok: bool,
    detail: serde_json::Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PrefetchReport {
    release: WindowsRelease,
    staged_path: String,
    downloaded: bool,
    size: u64,
    sha256: String,
    authenticode_status: String,
    authenticode_subject: String,
    identity_name: String,
    identity_version: String,
    identity_architecture: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SmokeReport {
    install_root: String,
    user_data_path: Option<String>,
    user_data_existed_before: bool,
    user_data_exists_after: bool,
    steps: Vec<StepReport>,
    final_status: WinInstallStatus,
}

fn settings_and_endpoints() -> (AppSettings, MirrorEndpoints) {
    let target = Target::current();
    let mirror_base_url = "https://codexapp.agentsmirror.com".to_string();
    let install_root = host::default_install_root(&target);
    let settings = AppSettings::new(mirror_base_url.clone(), install_root);
    let endpoints = MirrorEndpoints::from_base_url(&mirror_base_url);
    (settings, endpoints)
}

fn user_data_path() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .map(|home| home.join(".codex"))
}

fn staged_msix_path(release: &WindowsRelease) -> PathBuf {
    std::env::temp_dir()
        .join("codex-app-manager")
        .join("windows-staging")
        .join(format!("{}.msix", release.package_moniker))
}

fn old_release() -> WindowsRelease {
    WindowsRelease {
        version: OLD_MSIX_VERSION.to_string(),
        package_moniker: OLD_MSIX_MONIKER.to_string(),
        architecture: Some("x64".to_string()),
        content_length: Some(OLD_MSIX_SIZE),
        etag: None,
        store_product_id: Some("9PLM9XGG6VKS".to_string()),
        package_identity: Some("OpenAI.Codex".to_string()),
    }
}

fn prefetch_verified_release_msix(
    release: WindowsRelease,
    url: &str,
    expected_sha: &str,
) -> Result<PrefetchReport, String> {
    let dest = staged_msix_path(&release);
    let cached_ok = dest.exists()
        && sha256_file(&dest)
            .map(|actual| actual.eq_ignore_ascii_case(expected_sha))
            .unwrap_or(false);
    let mut downloaded = false;
    if !cached_ok {
        if dest.exists() {
            std::fs::remove_file(&dest)
                .map_err(|e| format!("remove stale staged MSIX {}: {e}", dest.display()))?;
        }
        download_to(url, &dest).map_err(|e| e.to_string())?;
        downloaded = true;
    }

    let size = std::fs::metadata(&dest)
        .map_err(|e| format!("read staged MSIX metadata {}: {e}", dest.display()))?
        .len();
    if let Some(expected_size) = release.content_length {
        if size != expected_size {
            return Err(format!(
                "MSIX size mismatch: actual {size}, expected {expected_size}"
            ));
        }
    }

    let actual_sha = sha256_file(&dest).map_err(|e| e.to_string())?;
    if !actual_sha.eq_ignore_ascii_case(expected_sha) {
        return Err(format!(
            "MSIX sha256 mismatch: actual {actual_sha}, expected {expected_sha}"
        ));
    }

    let authenticode = verify_openai_authenticode(Path::new(&dest)).map_err(|e| e.to_string())?;
    if !authenticode.is_valid_openai() {
        return Err(format!(
            "MSIX Authenticode verification failed: status={}, subject={}",
            authenticode.status, authenticode.subject
        ));
    }

    let identity = read_msix_identity(&dest).map_err(|e| e.to_string())?;
    validate_codex_identity(&identity, &release.version, release.architecture.as_deref())
        .map_err(|e| e.to_string())?;

    Ok(PrefetchReport {
        release,
        staged_path: dest.to_string_lossy().into_owned(),
        downloaded,
        size,
        sha256: actual_sha,
        authenticode_status: authenticode.status,
        authenticode_subject: authenticode.subject,
        identity_name: identity.name,
        identity_version: identity.version,
        identity_architecture: identity.processor_architecture,
    })
}

fn prefetch_verified_old_msix() -> Result<PrefetchReport, String> {
    prefetch_verified_release_msix(old_release(), OLD_MSIX_URL, OLD_MSIX_SHA256)
}

fn prefetch_verified_msix(endpoints: &MirrorEndpoints) -> Result<PrefetchReport, String> {
    let manifest_text = fetch_text(&endpoints.manifest_url).map_err(|e| e.to_string())?;
    let checksums_text = fetch_text(&endpoints.checksums_url).map_err(|e| e.to_string())?;
    let release = parse_manifest(&manifest_text).map_err(|e| e.to_string())?;
    let expected_sha =
        find_msix_sha256(&checksums_text, &release.package_moniker).map_err(|e| e.to_string())?;
    let dest = staged_msix_path(&release);

    let cached_ok = dest.exists()
        && sha256_file(&dest)
            .map(|actual| actual.eq_ignore_ascii_case(&expected_sha))
            .unwrap_or(false);
    let mut downloaded = false;
    if !cached_ok {
        if dest.exists() {
            std::fs::remove_file(&dest)
                .map_err(|e| format!("remove stale staged MSIX {}: {e}", dest.display()))?;
        }
        download_to(&endpoints.windows_msix_url, &dest).map_err(|e| e.to_string())?;
        downloaded = true;
    }

    let size = std::fs::metadata(&dest)
        .map_err(|e| format!("read staged MSIX metadata {}: {e}", dest.display()))?
        .len();
    if let Some(expected_size) = release.content_length {
        if size != expected_size {
            return Err(format!(
                "MSIX size mismatch: actual {size}, expected {expected_size}"
            ));
        }
    }

    let actual_sha = sha256_file(&dest).map_err(|e| e.to_string())?;
    if !actual_sha.eq_ignore_ascii_case(&expected_sha) {
        return Err(format!(
            "MSIX sha256 mismatch: actual {actual_sha}, expected {expected_sha}"
        ));
    }

    let authenticode = verify_openai_authenticode(Path::new(&dest)).map_err(|e| e.to_string())?;
    if !authenticode.is_valid_openai() {
        return Err(format!(
            "MSIX Authenticode verification failed: status={}, subject={}",
            authenticode.status, authenticode.subject
        ));
    }

    let identity = read_msix_identity(&dest).map_err(|e| e.to_string())?;
    validate_codex_identity(&identity, &release.version, release.architecture.as_deref())
        .map_err(|e| e.to_string())?;

    Ok(PrefetchReport {
        release,
        staged_path: dest.to_string_lossy().into_owned(),
        downloaded,
        size,
        sha256: actual_sha,
        authenticode_status: authenticode.status,
        authenticode_subject: authenticode.subject,
        identity_name: identity.name,
        identity_version: identity.version,
        identity_architecture: identity.processor_architecture,
    })
}

fn step<T: Serialize>(steps: &mut Vec<StepReport>, name: &str, detail: T) -> Result<(), String> {
    let detail = serde_json::to_value(detail).map_err(|e| format!("serialize {name}: {e}"))?;
    steps.push(StepReport {
        step: name.to_string(),
        ok: true,
        detail,
    });
    Ok(())
}

fn record_managed(path: String, version: &str, source: &str) -> Result<(), String> {
    let mut store = ProvenanceStore::load();
    store.record(path, version_key(version), source);
    store.save().map_err(|e| e.to_string())
}

fn remove_rollback(settings: &AppSettings) {
    let install_root = PathBuf::from(&settings.install_root);
    if let Some(parent) = install_root.parent() {
        let _ = std::fs::remove_dir_all(parent.join("Codex.rollback"));
    }
}

fn ensure_no_install(settings: &AppSettings, steps: &mut Vec<StepReport>) -> Result<(), String> {
    let status = win_install_status(settings);
    if status.status == "external" {
        let adopted = win_adopt(settings).map_err(|e| format!("adopt external install: {e}"))?;
        step(steps, "adopt-existing-external", &adopted)?;
    }
    if status.status != "none" {
        let uninstalled =
            uninstall_windows_codex(settings, true, false).map_err(|e| format!("{e}"))?;
        step(steps, "uninstall-existing-preserve-user-data", &uninstalled)?;
        remove_rollback(settings);
        let after_uninstall = win_install_status(settings);
        step(steps, "status-after-existing-uninstall", &after_uninstall)?;
        if after_uninstall.status != "none" {
            return Err(format!(
                "expected no install after uninstall, got {}",
                after_uninstall.status
            ));
        }
    }
    Ok(())
}

fn reinstall_final_msix(
    endpoints: &MirrorEndpoints,
    settings: &AppSettings,
    steps: &mut Vec<StepReport>,
) -> Result<WinInstallStatus, String> {
    let reinstalled = perform_windows_update(endpoints, settings, true)
        .map_err(|e| format!("reinstall final MSIX: {e}"))?;
    step(steps, "reinstall-final-msix", &reinstalled)?;
    let final_status = win_install_status(settings);
    step(steps, "final-status", &final_status)?;
    if final_status.status != "managed"
        || final_status.installed.as_ref().map(|i| i.source.as_str()) != Some("msix")
    {
        return Err(format!(
            "expected final managed MSIX install, got status={} source={}",
            final_status.status,
            final_status
                .installed
                .as_ref()
                .map(|i| i.source.as_str())
                .unwrap_or("none")
        ));
    }
    Ok(final_status)
}

fn run_install_uninstall_install() -> Result<SmokeReport, String> {
    let (settings, endpoints) = settings_and_endpoints();
    let user_data = user_data_path();
    let user_data_existed_before = user_data.as_ref().is_some_and(|path| path.exists());
    let mut steps = Vec::new();

    let initial = win_install_status(&settings);
    step(&mut steps, "initial-status", &initial)?;

    let prefetch = prefetch_verified_msix(&endpoints)?;
    step(&mut steps, "prefetch-verified-msix", &prefetch)?;

    if initial.status == "external" {
        let adopted = win_adopt(&settings).map_err(|e| format!("adopt external install: {e}"))?;
        step(&mut steps, "adopt-existing-external", &adopted)?;
    }

    if initial.status != "none" {
        let uninstalled =
            uninstall_windows_codex(&settings, true, false).map_err(|e| format!("{e}"))?;
        step(
            &mut steps,
            "uninstall-existing-preserve-user-data",
            &uninstalled,
        )?;
        let after_uninstall = win_install_status(&settings);
        step(
            &mut steps,
            "status-after-existing-uninstall",
            &after_uninstall,
        )?;
        if after_uninstall.status != "none" {
            return Err(format!(
                "expected no install after uninstall, got {}",
                after_uninstall.status
            ));
        }
    }

    let installed = perform_windows_update(&endpoints, &settings, true)
        .map_err(|e| format!("install after uninstall: {e}"))?;
    step(&mut steps, "install", &installed)?;
    let after_install = win_install_status(&settings);
    step(&mut steps, "status-after-install", &after_install)?;
    if after_install.status != "managed" {
        return Err(format!(
            "expected managed install after install, got {}",
            after_install.status
        ));
    }

    let uninstalled =
        uninstall_windows_codex(&settings, true, false).map_err(|e| format!("{e}"))?;
    step(
        &mut steps,
        "uninstall-installed-preserve-user-data",
        &uninstalled,
    )?;
    let after_second_uninstall = win_install_status(&settings);
    step(
        &mut steps,
        "status-after-installed-uninstall",
        &after_second_uninstall,
    )?;
    if after_second_uninstall.status != "none" {
        return Err(format!(
            "expected no install after second uninstall, got {}",
            after_second_uninstall.status
        ));
    }

    let reinstalled = perform_windows_update(&endpoints, &settings, true)
        .map_err(|e| format!("reinstall after uninstall: {e}"))?;
    step(&mut steps, "reinstall-final", &reinstalled)?;
    let final_status = win_install_status(&settings);
    step(&mut steps, "final-status", &final_status)?;
    if final_status.status != "managed" {
        return Err(format!(
            "expected final managed install, got {}",
            final_status.status
        ));
    }

    Ok(SmokeReport {
        install_root: settings.install_root,
        user_data_path: user_data
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        user_data_existed_before,
        user_data_exists_after: user_data.as_ref().is_some_and(|path| path.exists()),
        steps,
        final_status,
    })
}

fn run_force_portable_cycle() -> Result<SmokeReport, String> {
    let (settings, endpoints) = settings_and_endpoints();
    let user_data = user_data_path();
    let user_data_existed_before = user_data.as_ref().is_some_and(|path| path.exists());
    let mut steps = Vec::new();

    let initial = win_install_status(&settings);
    step(&mut steps, "initial-status", &initial)?;
    let prefetch = prefetch_verified_msix(&endpoints)?;
    step(&mut steps, "prefetch-verified-msix", &prefetch)?;
    ensure_no_install(&settings, &mut steps)?;

    let portable_install = install_portable_from_msix(
        Path::new(&prefetch.staged_path),
        Path::new(&settings.install_root),
        false,
    )
    .map_err(|e| format!("force portable install: {e}"))?;
    record_managed(
        settings.install_root.clone(),
        &portable_install.version,
        "smoke-forced-portable",
    )?;
    step(&mut steps, "force-portable-install", &portable_install)?;
    let after_install = win_install_status(&settings);
    step(&mut steps, "status-after-portable-install", &after_install)?;
    if after_install.status != "managed"
        || after_install.installed.as_ref().map(|i| i.source.as_str()) != Some("portable")
    {
        return Err("expected managed portable install after forced portable install".to_string());
    }

    let portable_update = install_portable_from_msix(
        Path::new(&prefetch.staged_path),
        Path::new(&settings.install_root),
        false,
    )
    .map_err(|e| format!("force portable update: {e}"))?;
    record_managed(
        settings.install_root.clone(),
        &portable_update.version,
        "smoke-forced-portable-update",
    )?;
    step(
        &mut steps,
        "force-portable-same-version-update",
        &portable_update,
    )?;
    let after_update = win_install_status(&settings);
    step(&mut steps, "status-after-portable-update", &after_update)?;
    if after_update.status != "managed"
        || after_update.installed.as_ref().map(|i| i.version.as_str())
            != Some(prefetch.release.version.as_str())
    {
        return Err("expected managed latest portable after forced portable update".to_string());
    }

    let uninstalled =
        uninstall_windows_codex(&settings, true, false).map_err(|e| format!("{e}"))?;
    step(
        &mut steps,
        "uninstall-forced-portable-preserve-user-data",
        &uninstalled,
    )?;
    remove_rollback(&settings);
    let after_uninstall = win_install_status(&settings);
    step(
        &mut steps,
        "status-after-forced-portable-uninstall",
        &after_uninstall,
    )?;
    if after_uninstall.status != "none" {
        return Err(format!(
            "expected no install after forced portable uninstall, got {}",
            after_uninstall.status
        ));
    }

    let final_status = reinstall_final_msix(&endpoints, &settings, &mut steps)?;

    Ok(SmokeReport {
        install_root: settings.install_root,
        user_data_path: user_data
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        user_data_existed_before,
        user_data_exists_after: user_data.as_ref().is_some_and(|path| path.exists()),
        steps,
        final_status,
    })
}

fn write_fake_old_portable(root: &Path) -> Result<(), String> {
    if root.exists() {
        std::fs::remove_dir_all(root).map_err(|e| format!("remove existing fake root: {e}"))?;
    }
    std::fs::create_dir_all(root.join("resources"))
        .map_err(|e| format!("create fake portable root: {e}"))?;
    std::fs::write(root.join("Codex.exe"), b"fake-old-codex")
        .map_err(|e| format!("write fake Codex.exe: {e}"))?;
    std::fs::write(
        root.join("AppxManifest.xml"),
        r#"<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10">
  <Identity Name="OpenAI.Codex"
            Publisher="CN=50BDFD77-8903-4850-9FFE-6E8522F64D5B"
            Version="0.0.1.0"
            ProcessorArchitecture="x64" />
</Package>"#,
    )
    .map_err(|e| format!("write fake AppxManifest.xml: {e}"))?;
    Ok(())
}

fn run_fake_old_portable_update() -> Result<SmokeReport, String> {
    let (settings, endpoints) = settings_and_endpoints();
    let user_data = user_data_path();
    let user_data_existed_before = user_data.as_ref().is_some_and(|path| path.exists());
    let mut steps = Vec::new();

    let initial = win_install_status(&settings);
    step(&mut steps, "initial-status", &initial)?;
    let prefetch = prefetch_verified_msix(&endpoints)?;
    step(&mut steps, "prefetch-verified-msix", &prefetch)?;
    ensure_no_install(&settings, &mut steps)?;

    let root = PathBuf::from(&settings.install_root);
    write_fake_old_portable(&root)?;
    record_managed(
        settings.install_root.clone(),
        "0.0.1.0",
        "smoke-fake-old-portable",
    )?;
    let old_status = win_install_status(&settings);
    step(&mut steps, "status-after-fake-old-portable", &old_status)?;
    if old_status.installed.as_ref().map(|i| i.version.as_str()) != Some("0.0.1.0") {
        return Err("expected fake old portable version 0.0.1.0".to_string());
    }

    let updated = install_portable_from_msix(
        Path::new(&prefetch.staged_path),
        Path::new(&settings.install_root),
        false,
    )
    .map_err(|e| format!("update fake old portable: {e}"))?;
    record_managed(
        settings.install_root.clone(),
        &updated.version,
        "smoke-fake-old-portable-updated",
    )?;
    step(&mut steps, "update-fake-old-portable-to-latest", &updated)?;
    let after_update = win_install_status(&settings);
    step(&mut steps, "status-after-fake-old-update", &after_update)?;
    if after_update.installed.as_ref().map(|i| i.version.as_str())
        != Some(prefetch.release.version.as_str())
    {
        return Err("expected fake old portable updated to latest".to_string());
    }

    let uninstalled =
        uninstall_windows_codex(&settings, true, false).map_err(|e| format!("{e}"))?;
    step(
        &mut steps,
        "uninstall-updated-fake-old-portable-preserve-user-data",
        &uninstalled,
    )?;
    remove_rollback(&settings);
    let after_uninstall = win_install_status(&settings);
    step(
        &mut steps,
        "status-after-fake-old-update-uninstall",
        &after_uninstall,
    )?;
    if after_uninstall.status != "none" {
        return Err(format!(
            "expected no install after fake old update uninstall, got {}",
            after_uninstall.status
        ));
    }

    let final_status = reinstall_final_msix(&endpoints, &settings, &mut steps)?;

    Ok(SmokeReport {
        install_root: settings.install_root,
        user_data_path: user_data
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        user_data_existed_before,
        user_data_exists_after: user_data.as_ref().is_some_and(|path| path.exists()),
        steps,
        final_status,
    })
}

fn run_old_msix_to_latest_msix() -> Result<SmokeReport, String> {
    let (settings, endpoints) = settings_and_endpoints();
    let user_data = user_data_path();
    let user_data_existed_before = user_data.as_ref().is_some_and(|path| path.exists());
    let mut steps = Vec::new();

    let initial = win_install_status(&settings);
    step(&mut steps, "initial-status", &initial)?;
    let old = prefetch_verified_old_msix()?;
    step(&mut steps, "prefetch-verified-old-msix", &old)?;
    let latest = prefetch_verified_msix(&endpoints)?;
    step(&mut steps, "prefetch-verified-latest-msix", &latest)?;
    ensure_no_install(&settings, &mut steps)?;

    let old_install =
        install_msix_sideload(Path::new(&old.staged_path)).map_err(|e| format!("{e}"))?;
    step(&mut steps, "install-old-msix", &old_install)?;
    if !old_install.success {
        return Err(format!("old MSIX install failed: {}", old_install.message));
    }
    let old_status = win_install_status(&settings);
    if let Some(installed) = &old_status.installed {
        record_managed(installed.path.clone(), &installed.version, "smoke-old-msix")?;
    }
    let old_status = win_install_status(&settings);
    step(&mut steps, "status-after-old-msix-install", &old_status)?;
    if old_status.status != "managed"
        || old_status.installed.as_ref().map(|i| i.version.as_str()) != Some(OLD_MSIX_VERSION)
    {
        return Err("expected managed old MSIX install before upgrade".to_string());
    }

    let upgraded = perform_windows_update(&endpoints, &settings, true)
        .map_err(|e| format!("upgrade old MSIX to latest: {e}"))?;
    step(&mut steps, "upgrade-old-msix-to-latest", &upgraded)?;
    if upgraded.action != "msix-sideload" {
        return Err(format!(
            "expected MSIX sideload upgrade action, got {}",
            upgraded.action
        ));
    }
    let final_status = win_install_status(&settings);
    step(&mut steps, "final-status", &final_status)?;
    if final_status.status != "managed"
        || final_status
            .installed
            .as_ref()
            .map(|i| (i.source.as_str(), i.version.as_str()))
            != Some(("msix", latest.release.version.as_str()))
    {
        return Err("expected final managed latest MSIX after old MSIX upgrade".to_string());
    }

    Ok(SmokeReport {
        install_root: settings.install_root,
        user_data_path: user_data
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        user_data_existed_before,
        user_data_exists_after: user_data.as_ref().is_some_and(|path| path.exists()),
        steps,
        final_status,
    })
}

fn run_old_portable_to_latest_portable() -> Result<SmokeReport, String> {
    let (settings, endpoints) = settings_and_endpoints();
    let user_data = user_data_path();
    let user_data_existed_before = user_data.as_ref().is_some_and(|path| path.exists());
    let mut steps = Vec::new();

    let initial = win_install_status(&settings);
    step(&mut steps, "initial-status", &initial)?;
    let old = prefetch_verified_old_msix()?;
    step(&mut steps, "prefetch-verified-old-msix", &old)?;
    let latest = prefetch_verified_msix(&endpoints)?;
    step(&mut steps, "prefetch-verified-latest-msix", &latest)?;
    ensure_no_install(&settings, &mut steps)?;

    let old_portable = install_portable_from_msix(
        Path::new(&old.staged_path),
        Path::new(&settings.install_root),
        false,
    )
    .map_err(|e| format!("install old portable: {e}"))?;
    record_managed(
        settings.install_root.clone(),
        &old_portable.version,
        "smoke-old-portable",
    )?;
    step(&mut steps, "install-old-portable", &old_portable)?;
    let old_status = win_install_status(&settings);
    step(&mut steps, "status-after-old-portable-install", &old_status)?;
    if old_status.status != "managed"
        || old_status
            .installed
            .as_ref()
            .map(|i| (i.source.as_str(), i.version.as_str()))
            != Some(("portable", OLD_MSIX_VERSION))
    {
        return Err("expected managed old portable install before upgrade".to_string());
    }

    let updated_portable = install_portable_from_msix(
        Path::new(&latest.staged_path),
        Path::new(&settings.install_root),
        false,
    )
    .map_err(|e| format!("update old portable to latest: {e}"))?;
    record_managed(
        settings.install_root.clone(),
        &updated_portable.version,
        "smoke-old-portable-updated",
    )?;
    step(
        &mut steps,
        "upgrade-old-portable-to-latest-portable",
        &updated_portable,
    )?;
    let after_update = win_install_status(&settings);
    step(
        &mut steps,
        "status-after-old-portable-upgrade",
        &after_update,
    )?;
    if after_update.status != "managed"
        || after_update
            .installed
            .as_ref()
            .map(|i| (i.source.as_str(), i.version.as_str()))
            != Some(("portable", latest.release.version.as_str()))
    {
        return Err("expected managed latest portable after old portable upgrade".to_string());
    }

    let uninstalled =
        uninstall_windows_codex(&settings, true, false).map_err(|e| format!("{e}"))?;
    step(
        &mut steps,
        "uninstall-upgraded-old-portable-preserve-user-data",
        &uninstalled,
    )?;
    remove_rollback(&settings);
    let after_uninstall = win_install_status(&settings);
    step(
        &mut steps,
        "status-after-old-portable-uninstall",
        &after_uninstall,
    )?;
    if after_uninstall.status != "none" {
        return Err(format!(
            "expected no install after old portable uninstall, got {}",
            after_uninstall.status
        ));
    }

    let final_status = reinstall_final_msix(&endpoints, &settings, &mut steps)?;

    Ok(SmokeReport {
        install_root: settings.install_root,
        user_data_path: user_data
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        user_data_existed_before,
        user_data_exists_after: user_data.as_ref().is_some_and(|path| path.exists()),
        steps,
        final_status,
    })
}

fn run() -> Result<serde_json::Value, String> {
    let (settings, endpoints) = settings_and_endpoints();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str).unwrap_or("status") {
        "status" => serde_json::to_value(win_install_status(&settings)).map_err(|e| e.to_string()),
        "prefetch" => {
            serde_json::to_value(prefetch_verified_msix(&endpoints)?).map_err(|e| e.to_string())
        }
        "install" => serde_json::to_value(
            perform_windows_update(&endpoints, &settings, true).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string()),
        "uninstall" => serde_json::to_value(
            uninstall_windows_codex(&settings, true, false).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string()),
        "install-uninstall-install" => {
            serde_json::to_value(run_install_uninstall_install()?).map_err(|e| e.to_string())
        }
        "force-portable-cycle" => {
            serde_json::to_value(run_force_portable_cycle()?).map_err(|e| e.to_string())
        }
        "fake-old-portable-update" => {
            serde_json::to_value(run_fake_old_portable_update()?).map_err(|e| e.to_string())
        }
        "old-msix-to-latest-msix" => {
            serde_json::to_value(run_old_msix_to_latest_msix()?).map_err(|e| e.to_string())
        }
        "old-portable-to-latest-portable" => {
            serde_json::to_value(run_old_portable_to_latest_portable()?)
                .map_err(|e| e.to_string())
        }
        other => Err(format!(
            "unknown command {other}; use status | prefetch | install | uninstall | install-uninstall-install | force-portable-cycle | fake-old-portable-update | old-msix-to-latest-msix | old-portable-to-latest-portable"
        )),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(value) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
            );
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}
