//! The OpenMW an instance runs with.
//!
//! Lists need a particular OpenMW, so each instance carries its own: MOMW's
//! portable AppImage build (engine, ini importer and navmesh tool of one
//! release) is downloaded once into a shared store and linked into the
//! instance. Nothing needs to be installed on the machine, and the navigation
//! meshes are always built by the same version that will read them.

use anyhow::{bail, Context, Result};
use futures::StreamExt;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

const APPIMAGE_API: &str = "https://gitlab.com/api/v4/projects/modding-openmw%2Fopenmw-appimage";
const ARCH: &str = "x86_64";

/// Which OpenMW release to use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Channel {
    /// The newest stable release (no release candidates).
    Stable,
    /// A specific published version, e.g. `0.51.0` or `dev-build`.
    Exact(String),
}

impl Channel {
    pub fn from_arg(version: Option<&str>) -> Self {
        match version.map(str::trim).filter(|v| !v.is_empty()) {
            Some(v) => Channel::Exact(v.to_owned()),
            None => Channel::Stable,
        }
    }
}

/// The three programs a list needs, unpacked in the shared store.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: String,
    pub dir: PathBuf,
}

const PROGRAMS: &[(&str, &str)] = &[
    ("Engine", "openmw"),
    ("Iniimporter", "openmw-iniimporter"),
    ("Navmeshtool", "openmw-navmeshtool"),
];

fn appimage_name(program: &str, version: &str) -> String {
    format!("OpenMW_{program}-{version}-{ARCH}.AppImage")
}

impl Release {
    pub fn appimage(&self, program: &str) -> PathBuf {
        self.dir.join(appimage_name(program, &self.version))
    }

    fn is_complete(&self) -> bool {
        PROGRAMS.iter().all(|(p, _)| self.appimage(p).is_file())
    }
}

/// Shared store: `<local data>/clf3/openmw/<version>/`.
pub fn default_store_dir() -> Result<PathBuf> {
    Ok(dirs::data_local_dir()
        .context("could not find the local data directory")?
        .join("ds2modpack")
        .join("openmw"))
}

#[derive(Deserialize)]
struct Package {
    id: u64,
    version: String,
}

#[derive(Deserialize)]
struct PackageFile {
    file_name: String,
}

fn is_stable(version: &str) -> bool {
    version.split('.').count() == 3 && version.split('.').all(|n| n.parse::<u32>().is_ok())
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!(
            "ds2modpack/",
            env!("CARGO_PKG_VERSION"),
            " (+https://github.com/himanshudbg/ds2-modpack-app)"
        ))
        .build()?)
}

/// Make sure the wanted release is in `store`, downloading it when it is not.
pub async fn ensure_release(store: &Path, channel: &Channel) -> Result<Release> {
    // An exact version that is already unpacked needs no network at all.
    if let Channel::Exact(version) = channel {
        let release = Release {
            version: version.clone(),
            dir: store.join(version),
        };
        if release.is_complete() {
            return Ok(release);
        }
    }
    let client = client()?;
    let packages: Vec<Package> = client
        .get(format!(
            "{APPIMAGE_API}/packages?per_page=50&order_by=created_at&sort=desc"
        ))
        .send()
        .await
        .context("looking up OpenMW releases")?
        .error_for_status()?
        .json()
        .await?;
    let package = match channel {
        Channel::Stable => packages.iter().find(|p| is_stable(&p.version)),
        Channel::Exact(v) => packages.iter().find(|p| &p.version == v),
    }
    .with_context(|| match channel {
        Channel::Stable => "no stable OpenMW release was found".to_owned(),
        Channel::Exact(v) => format!("OpenMW {v} is not published"),
    })?;
    let release = Release {
        version: package.version.clone(),
        dir: store.join(&package.version),
    };
    if release.is_complete() {
        return Ok(release);
    }

    let files: Vec<PackageFile> = client
        .get(format!(
            "{APPIMAGE_API}/packages/{}/package_files",
            package.id
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let tar = files
        .iter()
        .find(|f| f.file_name.ends_with(".tar"))
        .context("the OpenMW release has no archive")?;
    let sum = files
        .iter()
        .find(|f| f.file_name == format!("{}.sha512sum.txt", tar.file_name))
        .context("the OpenMW release has no checksum")?;
    let base = format!(
        "{APPIMAGE_API}/packages/generic/openmw-appimage/{}",
        package.version
    );
    let expected = client
        .get(format!("{base}/{}", sum.file_name))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?
        .split_whitespace()
        .next()
        .context("empty checksum file")?
        .to_lowercase();

    std::fs::create_dir_all(&release.dir)?;
    let archive = release.dir.join("download.partial");
    let result = async {
        use sha2::{Digest, Sha512};
        let response = client
            .get(format!("{base}/{}", tar.file_name))
            .send()
            .await?
            .error_for_status()?;
        let mut file = tokio::fs::File::create(&archive).await?;
        let mut hasher = Sha512::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("downloading OpenMW")?;
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        let actual: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if actual != expected {
            bail!("the OpenMW download does not match its published checksum");
        }
        let (archive_path, dest, version) = (
            archive.clone(),
            release.dir.clone(),
            release.version.clone(),
        );
        tokio::task::spawn_blocking(move || extract_programs(&archive_path, &dest, &version))
            .await?
    }
    .await;
    let _ = std::fs::remove_file(&archive);
    result?;
    if !release.is_complete() {
        bail!("the OpenMW release is missing a program");
    }
    Ok(release)
}

/// Keep the engine, ini importer and navmesh tool from the release archive.
fn extract_programs(archive: &Path, dest: &Path, version: &str) -> Result<()> {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    let wanted: Vec<String> = PROGRAMS
        .iter()
        .map(|(p, _)| appimage_name(p, version))
        .collect();
    let mut tar = tar::Archive::new(std::fs::File::open(archive)?);
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };
        if wanted.contains(&name) {
            let tmp = dest.join(format!("{name}.partial"));
            entry.unpack(&tmp)?;
        #[cfg(unix)]
         std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
            std::fs::rename(&tmp, dest.join(&name))?;
        }
    }
    Ok(())
}

/// Wrapper that runs an AppImage without needing FUSE on the machine. The
/// engine's and navmesh tool's wrappers also point OpenMW at this instance's
/// own profile when the caller gave no config, so Fluorine can start them with
/// no arguments.
fn wrapper_script(version: &str, program: &str) -> String {
    let engine = if matches!(program, "Engine" | "Navmeshtool") {
        "ROOT=$(dirname -- \"$HERE\")\n\
         case \" $* \" in\n\
         *\" --config \"*|*\" --replace=config \"*) ;;\n\
         *)\n\
         \x20   NAME=$(sed -n 's/^selected_profile=//p' \"$ROOT/ModOrganizer.ini\" 2>/dev/null | head -n 1)\n\
         \x20   PROFILE=\"${OPENMW_PROFILE:-$ROOT/profiles/${NAME:-Default}}\"\n\
         \x20   set -- --replace=config --config \"$PROFILE\" --user-data \"$PROFILE\" \"$@\" ;;\n\
         esac\n"
    } else {
        ""
    };
    // The engine is started far more often than the other programs, so it is
    // unpacked once next to the AppImage instead of on every launch. If that
    // is not possible it falls back to the AppImage's own extract-and-run.
    let unpack = if program == "Engine" {
        "UNPACKED=\"$HERE/unpacked\"\n\
         if [ ! -x \"$UNPACKED/AppRun\" ]; then\n\
         \x20   TMP=\"$HERE/.unpack.$$\"\n\
         \x20   mkdir -p \"$TMP\" && (cd \"$TMP\" && \"$HERE/{image}\" --appimage-extract >/dev/null 2>&1) \\\n\
         \x20       && mv \"$TMP/squashfs-root\" \"$UNPACKED\" 2>/dev/null\n\
         \x20   rm -rf \"$TMP\"\n\
         fi\n\
         [ -x \"$UNPACKED/AppRun\" ] && exec \"$UNPACKED/AppRun\" \"$@\"\n"
            .replace("{image}", &appimage_name(program, version))
    } else {
        String::new()
    };
    format!(
        "#!/bin/sh\n\
         # Runs the {program} bundled with this instance (OpenMW {version}).\n\
         HERE=$(CDPATH= cd -- \"$(dirname -- \"$0\")/..\" && pwd)\n\
         {engine}{unpack}export APPIMAGE_EXTRACT_AND_RUN=1\n\
         exec \"$HERE/{}\" \"$@\"\n",
        appimage_name(program, version)
    )
}

/// The same for every instance: starts the bundled OpenMW on the profile's
/// config, after repairing the paths in it if the folder was moved.
pub const LAUNCH_SCRIPT: &str = r#"#!/bin/sh
# Starts this instance's OpenMW with its own configuration, saves and
# navigation meshes. Nothing else needs to be installed.
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
NAME=$(sed -n 's/^selected_profile=//p' "$ROOT/ModOrganizer.ini" 2>/dev/null | head -n 1)
PROFILE="${OPENMW_PROFILE:-$ROOT/profiles/${NAME:-Default}}"
CFG="$PROFILE/openmw.cfg"

# The config stores absolute paths. If this folder was moved, point them here.
OLD=$(awk '/^data=.*\/mods\// {
    sub(/^data="?/, ""); i = index($0, "/mods/"); print substr($0, 1, i - 1); exit
}' "$CFG" 2>/dev/null | sed 's/&\(.\)/\1/g')
if [ -n "$OLD" ] && [ "$OLD" != "$ROOT" ]; then
    awk -v o="$(printf '%s' "$OLD" | sed 's/[&"]/&&/g')" \
        -v n="$(printf '%s' "$ROOT" | sed 's/[&"]/&&/g')" '
        /^(data|user-data|resources)=/ && (i = index($0, o)) > 0 {
            $0 = substr($0, 1, i - 1) n substr($0, i + length(o))
        }
        { print }' "$CFG" > "$CFG.tmp" && mv "$CFG.tmp" "$CFG"
fi

exec "$ROOT/openmw/bin/openmw" --replace=config --config "$PROFILE" \
    --user-data "$PROFILE" "$@"
"#;

/// Put the release into `root/openmw/` and write the launch script. The
/// programs are hard-linked from the store when possible (no extra disk) and
/// copied otherwise.
pub fn bundle(root: &Path, release: &Release) -> Result<()> {
    #[cfg(unix)]
     use std::os::unix::fs::PermissionsExt;
    let dir = root.join("openmw");
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin)?;
    for (program, command) in PROGRAMS {
        let source = release.appimage(program);
        let target = dir.join(appimage_name(program, &release.version));
        let _ = std::fs::remove_file(&target);
        if std::fs::hard_link(&source, &target).is_err() {
            std::fs::copy(&source, &target)
                .with_context(|| format!("copying {}", source.display()))?;
        }
        let wrapper = bin.join(command);
        std::fs::write(&wrapper, wrapper_script(&release.version, program))?;
        #[cfg(unix)]
         std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755))?;
    }
    // Leave only this release's programs behind (an upgrade replaces them).
    for entry in std::fs::read_dir(&dir)?.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".AppImage")
            && !PROGRAMS
                .iter()
                .any(|(p, _)| name == appimage_name(p, &release.version))
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    // A stale unpacked engine would keep running the old version.
    let _ = std::fs::remove_dir_all(dir.join("unpacked"));
    std::fs::write(dir.join("VERSION"), format!("{}\n", release.version))?;
    let launcher = root.join("Launch OpenMW.sh");
    std::fs::write(&launcher, LAUNCH_SCRIPT)?;
    #[cfg(unix)]
     std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

/// The text of a menu entry that starts the instance without Fluorine.
fn desktop_entry(root: &Path, title: &str) -> String {
    // Exec needs quoting for spaces; a double quote or backslash is escaped.
    let launcher = root
        .join("Launch OpenMW.sh")
        .to_string_lossy()
        .replace('\\', "\\\\\\\\")
        .replace('"', "\\\\\"")
        .replace('$', "\\\\$")
        .replace('`', "\\\\`");
    let path = root.to_string_lossy().replace('\n', " ");
    format!(
        "[Desktop Entry]\nType=Application\nName={title} (OpenMW)\n\
         Comment=Play this Modding-OpenMW list with its own bundled OpenMW\n\
         Exec=\"{launcher}\"\nPath={path}\nIcon=openmw\nTerminal=false\n\
         Categories=Game;\nStartupWMClass=openmw\n"
    )
}

/// Add the instance to the desktop's application menu. `applications` is
/// normally `~/.local/share/applications`; returns the file written.
pub fn install_desktop_entry(root: &Path, title: &str, applications: &Path) -> Result<PathBuf> {
    #[cfg(unix)]
     use std::os::unix::fs::PermissionsExt;
    let slug: String = root
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    std::fs::create_dir_all(applications)?;
    let file = applications.join(format!("ds2modpack-openmw-{}.desktop", slug.trim_matches('-')));
    std::fs::write(&file, desktop_entry(root, title))?;
    #[cfg(unix)]
     std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755))?;
    Ok(file)
}

/// `~/.local/share/applications` (or `$XDG_DATA_HOME/applications`).
pub fn default_applications_dir() -> Result<PathBuf> {
    if let Some(data) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(data).join("applications"));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".local/share/applications"))
}

/// The version bundled in an instance, if any.
pub fn bundled_version(root: &Path) -> Option<String> {
    std::fs::read_to_string(root.join("openmw").join("VERSION"))
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// A bundled program's wrapper.
pub fn program(root: &Path, command: &str) -> PathBuf {
    root.join("openmw").join("bin").join(command)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn channels_come_from_the_command_line() {
        assert_eq!(Channel::from_arg(None), Channel::Stable);
        assert_eq!(Channel::from_arg(Some(" ")), Channel::Stable);
        assert_eq!(
            Channel::from_arg(Some("0.50.0")),
            Channel::Exact("0.50.0".into())
        );
        assert!(is_stable("0.51.0") && !is_stable("0.51.0-RC2") && !is_stable("dev-build"));
    }

    fn fake_release(dir: &Path) -> Release {
        let release = Release {
            version: "0.51.0".into(),
            dir: dir.join("store/0.51.0"),
        };
        std::fs::create_dir_all(&release.dir).unwrap();
        for (program, _) in PROGRAMS {
            std::fs::write(release.appimage(program), program).unwrap();
        }
        release
    }

    #[test]
    fn bundling_links_programs_and_writes_wrappers() {
        let dir = tempfile::tempdir().unwrap();
        let release = fake_release(dir.path());
        assert!(release.is_complete());
        let root = dir.path().join("inst");
        // An older bundled release is replaced.
        std::fs::create_dir_all(root.join("openmw")).unwrap();
        std::fs::write(
            root.join("openmw/OpenMW_Engine-0.50.0-x86_64.AppImage"),
            "old",
        )
        .unwrap();
        bundle(&root, &release).unwrap();

        assert!(root
            .join("openmw/OpenMW_Engine-0.51.0-x86_64.AppImage")
            .is_file());
        assert!(!root
            .join("openmw/OpenMW_Engine-0.50.0-x86_64.AppImage")
            .exists());
        assert_eq!(bundled_version(&root).as_deref(), Some("0.51.0"));
        let wrapper = std::fs::read_to_string(program(&root, "openmw")).unwrap();
        assert!(wrapper.contains("APPIMAGE_EXTRACT_AND_RUN=1"));
        assert!(wrapper.contains("OpenMW_Engine-0.51.0-x86_64.AppImage"));
        for command in ["openmw", "openmw-iniimporter", "openmw-navmeshtool"] {
            let mode = std::fs::metadata(program(&root, command))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "{command}");
        }
        assert_eq!(
            std::fs::metadata(root.join("Launch OpenMW.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0o111
        );
    }

    /// Runs the real launch script against a fake bundle, before and after the
    /// instance folder is moved.
    #[test]
    fn the_launch_script_repairs_paths_after_a_move() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old & place");
        let profile = old.join("profiles/Default");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::create_dir_all(old.join("openmw/bin")).unwrap();
        // The real wrapper, around a stand-in AppImage that records its arguments.
        let engine = old.join("openmw/OpenMW_Engine-0.51.0-x86_64.AppImage");
        std::fs::write(&engine, "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$0.args\"\n").unwrap();
        std::fs::set_permissions(&engine, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(
            old.join("openmw/bin/openmw"),
            wrapper_script("0.51.0", "Engine"),
        )
        .unwrap();
        std::fs::set_permissions(
            old.join("openmw/bin/openmw"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        std::fs::write(old.join("Launch OpenMW.sh"), LAUNCH_SCRIPT).unwrap();
        let quoted = quote_for_test(&old.to_string_lossy());
        std::fs::write(
            profile.join("openmw.cfg"),
            format!(
                "fallback=Foo,bar\ndata=\"/games/Morrowind/Data Files\"\ndata=\"{quoted}/mods/A\"\ndata=\"{quoted}/mods/B\"\n"
            ),
        )
        .unwrap();

        let moved = dir.path().join("moved");
        std::fs::rename(&old, &moved).unwrap();
        let status = std::process::Command::new("sh")
            .arg(moved.join("Launch OpenMW.sh"))
            .arg("--extra")
            .status()
            .unwrap();
        assert!(status.success());
        let cfg = std::fs::read_to_string(moved.join("profiles/Default/openmw.cfg")).unwrap();
        let moved_text = moved.to_string_lossy().into_owned();
        assert!(
            cfg.contains(&format!("data=\"{moved_text}/mods/A\"")),
            "{cfg}"
        );
        assert!(
            cfg.contains(&format!("data=\"{moved_text}/mods/B\"")),
            "{cfg}"
        );
        // The game folder is outside the instance and is left alone.
        assert!(
            cfg.contains("data=\"/games/Morrowind/Data Files\""),
            "{cfg}"
        );
        assert!(cfg.contains("fallback=Foo,bar"));
        let args =
            std::fs::read_to_string(moved.join("openmw/OpenMW_Engine-0.51.0-x86_64.AppImage.args"))
                .unwrap();
        assert!(args.contains("--replace=config"), "{args}");
        assert!(
            args.contains(&format!("{moved_text}/profiles/Default")),
            "{args}"
        );
        assert!(args.contains("--extra"), "{args}");
    }

    #[test]
    fn the_engine_wrapper_selects_the_instances_profile_unless_told_otherwise() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("inst");
        std::fs::create_dir_all(root.join("openmw/bin")).unwrap();
        let engine = root.join("openmw/OpenMW_Engine-0.51.0-x86_64.AppImage");
        std::fs::write(&engine, "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$0.args\"\n").unwrap();
        std::fs::set_permissions(&engine, std::fs::Permissions::from_mode(0o755)).unwrap();
        let wrapper = root.join("openmw/bin/openmw");
        std::fs::write(&wrapper, wrapper_script("0.51.0", "Engine")).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(
            root.join("ModOrganizer.ini"),
            "[General]\nselected_profile=My Profile\n",
        )
        .unwrap();

        let run = |extra: &[&str]| {
            assert!(std::process::Command::new(&wrapper)
                .args(extra)
                .status()
                .unwrap()
                .success());
            std::fs::read_to_string(format!("{}.args", engine.display())).unwrap()
        };
        let plain = run(&["--skip-menu"]);
        let profile = format!("{}/profiles/My Profile", root.display());
        assert!(
            plain.contains("--replace=config") && plain.contains(&profile),
            "{plain}"
        );
        assert!(plain.contains("--skip-menu"));
        // An explicit config is respected.
        let explicit = run(&["--config", "/elsewhere"]);
        assert!(
            !explicit.contains("--replace=config") && explicit.contains("/elsewhere"),
            "{explicit}"
        );
    }

    #[test]
    fn the_engine_is_unpacked_once_and_then_run_from_the_unpacked_copy() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("inst");
        std::fs::create_dir_all(root.join("openmw/bin")).unwrap();
        let image = root.join("openmw/OpenMW_Engine-0.51.0-x86_64.AppImage");
        // Like a real AppImage: --appimage-extract writes ./squashfs-root.
        std::fs::write(
            &image,
            "#!/bin/sh\n\
             if [ \"$1\" = --appimage-extract ]; then\n\
             mkdir squashfs-root && echo unpacked >> \"$(dirname \"$0\")/count\" && \
             printf '#!/bin/sh\\necho from-unpacked \"$@\" > \"$(dirname \"$0\")/../ran\"\\n' \
             > squashfs-root/AppRun && chmod +x squashfs-root/AppRun\n\
             else echo from-image > \"$(dirname \"$0\")/ran\"; fi\n",
        )
        .unwrap();
        std::fs::set_permissions(&image, std::fs::Permissions::from_mode(0o755)).unwrap();
        let wrapper = root.join("openmw/bin/openmw");
        std::fs::write(&wrapper, wrapper_script("0.51.0", "Engine")).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();

        for _ in 0..2 {
            assert!(std::process::Command::new(&wrapper)
                .args(["--config", "/x", "--skip-menu"])
                .status()
                .unwrap()
                .success());
        }
        let ran = std::fs::read_to_string(root.join("openmw/ran")).unwrap();
        assert!(ran.starts_with("from-unpacked"), "{ran}");
        assert!(ran.contains("--skip-menu"), "{ran}");
        let count = std::fs::read_to_string(root.join("openmw/count")).unwrap();
        assert_eq!(count.lines().count(), 1, "unpacked more than once");
    }

    #[test]
    fn the_menu_entry_starts_the_launcher_and_quotes_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("My List");
        std::fs::create_dir_all(&root).unwrap();
        let apps = dir.path().join("apps");
        let file = install_desktop_entry(&root, "My List", &apps).unwrap();
        assert_eq!(file.file_name().unwrap(), "ds2modpack-openmw-my-list.desktop");
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("Name=My List (OpenMW)"), "{text}");
        assert!(
            text.contains(&format!("Exec=\"{}/Launch OpenMW.sh\"", root.display())),
            "{text}"
        );
        assert!(text.contains(&format!("Path={}", root.display())));
    }

    #[test]
    fn the_navmesh_tool_defaults_to_the_profile_but_is_not_unpacked() {
        let navmesh = wrapper_script("0.51.0", "Navmeshtool");
        assert!(navmesh.contains("--replace=config"));
        assert!(!navmesh.contains("--appimage-extract"));
        let importer = wrapper_script("0.51.0", "Iniimporter");
        assert!(!importer.contains("--replace=config"));
    }

    fn quote_for_test(path: &str) -> String {
        path.replace('&', "&&").replace('"', "&\"")
    }
}
