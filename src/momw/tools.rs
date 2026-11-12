//! The helper programs a MOMW list needs after its mods are in place.
//!
//! The binaries come from the MOMW Tools Pack, which modding-openmw.com
//! publishes for Linux (delta_plugin, gocoverify, S3LightFixes,
//! openmw-validator, tes3cmd). They are downloaded at run time and are not
//! redistributed. `openmw-navmeshtool` comes with the user's OpenMW install.
//!
//! Generated plugins and meshes are written into one mod folder (the Tools
//! Pack's data directory), as the Configurator does for a Windows layout.

use anyhow::{bail, Context, Result};
use futures::StreamExt;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;

pub const TOOLS_PACK_URL: &str = "https://gitlab.com/api/v4/projects/modding-openmw%2Fmomw-tools-pack/jobs/artifacts/master/raw/momw-tools-pack-linux.tar.gz?job=make";

const DELTA_MERGED: &str = "delta-merged.omwaddon";
const DELETED_GROUNDCOVER: &str = "deleted_groundcover.omwaddon";
const LIGHT_FIXES: &str = "S3LightFixes.omwaddon";

/// Plugins delta_plugin must not merge: scripted content and its own output.
const DELTA_EXCLUDED: &[&str] = &[
    "LuaMultiMark.omwaddon",
    "S3maphore.esp",
    LIGHT_FIXES,
    DELTA_MERGED,
    DELETED_GROUNDCOVER,
];

/// Files taken from the pack, as `(name in the pack, name on disk)`.
fn wanted_files() -> Result<Vec<(String, &'static str)>> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        // delta_plugin, S3LightFixes and tes3cmd ship for x86-64 only.
        other => bail!("the MOMW Tools Pack has no {other} build of its tools"),
    };
    Ok(vec![
        ("delta_plugin".into(), "delta_plugin"),
        ("s3lightfixes".into(), "s3lightfixes"),
        ("tes3cmd".into(), "tes3cmd"),
        (format!("gocoverify-linux-{arch}"), "gocoverify"),
        (format!("openmw-validator-linux-{arch}"), "openmw-validator"),
        ("version.txt".into(), "version.txt"),
    ])
}

/// Where the Tools Pack binaries were unpacked.
#[derive(Debug, Clone)]
pub struct ToolSet {
    pub dir: PathBuf,
}

impl ToolSet {
    pub fn delta_plugin(&self) -> PathBuf {
        self.dir.join("delta_plugin")
    }
    pub fn gocoverify(&self) -> PathBuf {
        self.dir.join("gocoverify")
    }
    pub fn s3lightfixes(&self) -> PathBuf {
        self.dir.join("s3lightfixes")
    }
    pub fn validator(&self) -> PathBuf {
        self.dir.join("openmw-validator")
    }
    pub fn tes3cmd(&self) -> PathBuf {
        self.dir.join("tes3cmd")
    }

    /// The pack's version file, which lists every bundled tool's version.
    pub fn versions(&self) -> String {
        std::fs::read_to_string(self.dir.join("version.txt")).unwrap_or_default()
    }

    /// Whether every binary is present.
    pub fn is_complete(&self) -> bool {
        [
            self.delta_plugin(),
            self.gocoverify(),
            self.s3lightfixes(),
            self.validator(),
            self.tes3cmd(),
        ]
        .iter()
        .all(|p| p.is_file())
    }
}

/// Default location for the unpacked tools.
pub fn default_tools_dir() -> Result<PathBuf> {
    Ok(dirs::data_local_dir()
        .context("could not find the local data directory")?
        .join("modyssey")
        .join("tools")
        .join("momw-tools-pack"))
}

/// Make sure the tools are on disk, downloading the Tools Pack when they are
/// not (or when `force` is set).
pub async fn provision(dir: &Path, force: bool) -> Result<ToolSet> {
    let tools = ToolSet {
        dir: dir.to_path_buf(),
    };
    if !force && tools.is_complete() {
        return Ok(tools);
    }
    let wanted = wanted_files()?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;

    // Stream the ~150 MB archive to disk first; extracting needs a file.
    let archive = dir.join("momw-tools-pack.download");
    let result = async {
        let response = reqwest::Client::builder()
            .user_agent(concat!(
                "modyssey/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/himanshudbg/modyssey)"
            ))
            .build()?
            .get(TOOLS_PACK_URL)
            .send()
            .await
            .context("downloading the MOMW Tools Pack")?
            .error_for_status()
            .context("the MOMW Tools Pack download was refused")?;
        let mut file = tokio::fs::File::create(&archive).await?;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            file.write_all(&chunk.context("reading the MOMW Tools Pack")?)
                .await?;
        }
        file.flush().await?;
        drop(file);
        let (archive_path, dest) = (archive.clone(), dir.to_path_buf());
        tokio::task::spawn_blocking(move || extract_wanted(&archive_path, &dest, &wanted)).await?
    }
    .await;
    let _ = std::fs::remove_file(&archive);
    result?;

    if !tools.is_complete() {
        bail!("the MOMW Tools Pack did not contain every expected tool");
    }
    Ok(tools)
}

/// Unpack only the named files, flat, and mark binaries executable.
fn extract_wanted(archive: &Path, dest: &Path, wanted: &[(String, &'static str)]) -> Result<()> {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    let file = std::fs::File::open(archive)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        // Entries are at the archive root; anything nested is not a tool.
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if path.components().count() != 1 {
            continue;
        }
        let Some((_, target)) = wanted.iter().find(|(pack_name, _)| pack_name == name) else {
            continue;
        };
        let out = dest.join(target);
        let tmp = dest.join(format!("{target}.partial"));
        entry
            .unpack(&tmp)
            .with_context(|| format!("unpacking {name}"))?;
        if *target != "version.txt" {
            #[cfg(unix)]
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::rename(&tmp, &out)?;
    }
    Ok(())
}

/// One program invocation.
#[derive(Debug, Clone)]
pub struct ToolCommand {
    pub label: &'static str,
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(OsString, OsString)>,
}

impl ToolCommand {
    fn new(label: &'static str, program: PathBuf, args: Vec<OsString>) -> Self {
        Self {
            label,
            program,
            args,
            cwd: None,
            env: Vec::new(),
        }
    }
}

fn os<S: Into<OsString>>(s: S) -> OsString {
    s.into()
}

/// `openmw.cfg` text without the `content=` lines for `names` (any case).
pub fn without_content(cfg: &str, names: &[&str]) -> String {
    let mut out = String::with_capacity(cfg.len());
    for line in cfg.lines() {
        let drop = line
            .trim()
            .strip_prefix("content=")
            .is_some_and(|value| names.iter().any(|n| n.eq_ignore_ascii_case(value.trim())));
        if !drop {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

pub fn delta_merge_command(tools: &ToolSet, cfg: &Path, output_dir: &Path) -> ToolCommand {
    ToolCommand::new(
        "Delta Plugin",
        tools.delta_plugin(),
        vec![
            os("--verbose"),
            os("-c"),
            cfg.into(),
            os("merge"),
            os("--skip"),
            os("Cell"),
            os("--skip"),
            os("Dialogue"),
            os("--skip"),
            os("DialogueInfo"),
            output_dir.join(DELTA_MERGED).into(),
        ],
    )
}

/// GoCoverify keeps its settings in `$XDG_CONFIG_HOME/openmw/groundcoverify.toml`
/// and creates a default there. `settings_home` stands in for that variable so
/// the file lives with the instance instead of in the user's OpenMW config.
pub fn groundcoverify_command(
    tools: &ToolSet,
    cfg: &Path,
    settings_home: &Path,
    output_dir: &Path,
) -> ToolCommand {
    let mut args = vec![
        os("--openmw-config"),
        cfg.into(),
        os("--output-dir"),
        output_dir.into(),
    ];
    for ignored in ["LuaMultiMark.omwaddon", DELETED_GROUNDCOVER, LIGHT_FIXES] {
        args.push(os("--ignore"));
        args.push(os(ignored));
    }
    let mut command = ToolCommand::new("GoCoverify", tools.gocoverify(), args);
    command
        .env
        .push((os("XDG_CONFIG_HOME"), settings_home.into()));
    command
}

pub fn light_fixes_command(tools: &ToolSet, cfg: &Path, output_dir: &Path) -> ToolCommand {
    ToolCommand::new(
        "S3LightFixes",
        tools.s3lightfixes(),
        vec![
            os("--openmw-cfg"),
            cfg.into(),
            os("--output"),
            output_dir.into(),
            os("--no-notifications"),
        ],
    )
}

pub fn validator_command(tools: &ToolSet, cfg: &Path) -> ToolCommand {
    ToolCommand::new(
        "OpenMW-Validator",
        tools.validator(),
        vec![
            os("--cfg"),
            cfg.into(),
            os("--exit-nonzero"),
            os("--stdout"),
        ],
    )
}

/// The bundled `openmw-navmeshtool`. It reads the `openmw.cfg` in
/// `config_dir` and writes its cache (`navmesh.db`) to `user_data_dir`; both
/// are the profile folder, which is also where the game looks. Slow on large
/// lists.
pub fn navmeshtool_command(
    program: &Path,
    config_dir: &Path,
    user_data_dir: &Path,
    threads: usize,
) -> ToolCommand {
    ToolCommand::new(
        "OpenMW Navmeshtool",
        program.to_path_buf(),
        vec![
            os("--remove-unused-tiles"),
            os("--threads"),
            os(threads.to_string()),
            os("--replace=config"),
            os("--config"),
            config_dir.into(),
            os("--user-data"),
            user_data_dir.into(),
        ],
    )
}

/// The bundled `openmw-iniimporter`: turns Morrowind.ini into config lines.
pub fn iniimporter_command(program: &Path, ini: &Path, out_cfg: &Path) -> ToolCommand {
    ToolCommand::new(
        "OpenMW ini importer",
        program.to_path_buf(),
        vec![os("--ini"), ini.into(), os("--cfg"), out_cfg.into()],
    )
}

/// A list's `clean` action. No arguments means a plain `tes3cmd clean`, which
/// writes `Clean_<plugin>` next to the plugin. Relative `--backup-dir` values
/// are anchored in `backup_root` so backups never land in a mod folder.
pub fn tes3cmd_command(
    tools: &ToolSet,
    plugin: &Path,
    arguments: &[String],
    backup_root: &Path,
    game_data_dir: &Path,
) -> ToolCommand {
    let mut args: Vec<OsString> = Vec::new();
    if arguments.is_empty() {
        args.push(os("clean"));
    }
    let mut iter = arguments.iter();
    while let Some(argument) = iter.next() {
        args.push(os(argument));
        if argument == "--backup-dir" {
            if let Some(dir) = iter.next() {
                let dir = Path::new(dir);
                args.push(if dir.is_absolute() {
                    dir.into()
                } else {
                    backup_root.join(dir).into()
                });
            }
        }
    }
    args.push(plugin.into());
    let mut command = ToolCommand::new("tes3cmd", tools.tes3cmd(), args);
    // tes3cmd finds the masters through the "Data Files" folder above its
    // working directory.
    command.cwd = Some(game_data_dir.to_path_buf());
    command
}

/// Run a tool, passing each output line to `on_line`. The last lines are kept
/// for the error message.
pub async fn run(
    command: &ToolCommand,
    cancel: &CancellationToken,
    on_line: &(dyn Fn(&str) + Send + Sync),
) -> Result<()> {
    let mut process = tokio::process::Command::new(&command.program);
    process
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    process.envs(command.env.iter().map(|(k, v)| (k, v)));
    if let Some(cwd) = &command.cwd {
        process.current_dir(cwd);
    }
    let mut child = process
        .spawn()
        .with_context(|| format!("starting {} ({})", command.label, command.program.display()))?;
    let stdout = child.stdout.take().context("tool stdout")?;
    let stderr = child.stderr.take().context("tool stderr")?;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    for reader in [
        tokio::spawn(forward(BufReader::new(stdout), tx.clone())),
        tokio::spawn(forward(BufReader::new(stderr), tx.clone())),
    ] {
        drop(reader);
    }
    drop(tx);

    let mut tail: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    let status = loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = child.kill().await;
                bail!("{} was cancelled", command.label);
            }
            line = rx.recv() => match line {
                Some(line) => {
                    on_line(&line);
                    if tail.len() == 30 {
                        tail.pop_front();
                    }
                    tail.push_back(line);
                }
                None => break child.wait().await?,
            }
        }
    };
    if !status.success() {
        bail!(
            "{} failed ({status}):\n{}",
            command.label,
            tail.into_iter().collect::<Vec<_>>().join("\n")
        );
    }
    Ok(())
}

async fn forward<R: tokio::io::AsyncRead + Unpin>(
    reader: BufReader<R>,
    tx: tokio::sync::mpsc::UnboundedSender<String>,
) {
    let mut lines = reader.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if tx.send(line).is_err() {
            break;
        }
    }
}

/// What the post-install generators should do for a list.
#[derive(Debug, Clone, Default)]
pub struct GeneratorPlan {
    pub delta_plugin: bool,
    pub groundcoverify: bool,
    pub light_fixes: bool,
}

impl GeneratorPlan {
    pub fn from_tools(tools: &std::collections::BTreeSet<super::plan::Tool>) -> Self {
        use super::plan::Tool;
        Self {
            delta_plugin: tools.contains(&Tool::DeltaPlugin),
            groundcoverify: tools.contains(&Tool::Groundcoverify),
            light_fixes: tools.contains(&Tool::S3LightFixes),
        }
    }
}

/// Run delta_plugin, gocoverify and S3LightFixes in the Configurator's order.
///
/// `cfg` is the instance's full `openmw.cfg`, which is never edited. Derived
/// copies without the plugins a tool must not see are written to `config_dir`
/// together with the tools' own settings (`xdg-config/openmw/groundcoverify.toml`, and the
/// `lightConfig.toml` S3LightFixes keeps beside the cfg it is given). That
/// directory belongs to the instance, so those settings persist between runs
/// and nothing is written to the user's OpenMW config directory.
pub async fn run_generators(
    tools: &ToolSet,
    plan: &GeneratorPlan,
    cfg: &Path,
    output_dir: &Path,
    config_dir: &Path,
    cancel: &CancellationToken,
    on_line: &(dyn Fn(&str) + Send + Sync),
) -> Result<()> {
    std::fs::create_dir_all(output_dir)?;
    std::fs::create_dir_all(config_dir)?;
    let text =
        std::fs::read_to_string(cfg).with_context(|| format!("reading {}", cfg.display()))?;

    if plan.delta_plugin {
        let derived = config_dir.join("delta-openmw.cfg");
        std::fs::write(&derived, without_content(&text, DELTA_EXCLUDED))?;
        run(
            &delta_merge_command(tools, &derived, output_dir),
            cancel,
            on_line,
        )
        .await?;
    }
    if plan.groundcoverify {
        let settings_home = config_dir.join("xdg-config");
        run(
            &groundcoverify_command(tools, cfg, &settings_home, output_dir),
            cancel,
            on_line,
        )
        .await?;
    }
    if plan.light_fixes {
        let derived = config_dir.join("lightfixes-openmw.cfg");
        std::fs::write(&derived, without_content(&text, &[LIGHT_FIXES]))?;
        run(
            &light_fixes_command(tools, &derived, output_dir),
            cancel,
            on_line,
        )
        .await?;
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn tools() -> ToolSet {
        ToolSet {
            dir: PathBuf::from("/t"),
        }
    }

    fn strings(command: &ToolCommand) -> Vec<String> {
        command
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn content_lines_are_dropped_case_insensitively() {
        let cfg = "data=/a\ncontent=Morrowind.esm\ncontent=S3LightFixes.omwaddon\n\
                   content=DELTA-MERGED.omwaddon\ncontent=Other.esp\n";
        let out = without_content(cfg, DELTA_EXCLUDED);
        assert_eq!(out, "data=/a\ncontent=Morrowind.esm\ncontent=Other.esp\n");
    }

    #[test]
    fn delta_merge_skips_cells_and_dialogue() {
        let c = delta_merge_command(&tools(), Path::new("/w/c.cfg"), Path::new("/out"));
        assert_eq!(
            strings(&c),
            [
                "--verbose",
                "-c",
                "/w/c.cfg",
                "merge",
                "--skip",
                "Cell",
                "--skip",
                "Dialogue",
                "--skip",
                "DialogueInfo",
                "/out/delta-merged.omwaddon"
            ]
        );
        assert_eq!(c.program, PathBuf::from("/t/delta_plugin"));
    }

    #[test]
    fn groundcoverify_ignores_generated_plugins() {
        let c = groundcoverify_command(
            &tools(),
            Path::new("/c"),
            Path::new("/cfg/xdg"),
            Path::new("/out"),
        );
        let a = strings(&c);
        assert_eq!(&a[..4], ["--openmw-config", "/c", "--output-dir", "/out"]);
        assert_eq!(
            c.env,
            [(
                OsString::from("XDG_CONFIG_HOME"),
                OsString::from("/cfg/xdg")
            )]
        );
        for name in [
            "LuaMultiMark.omwaddon",
            "deleted_groundcover.omwaddon",
            "S3LightFixes.omwaddon",
        ] {
            let i = a.iter().position(|x| x == name).unwrap();
            assert_eq!(a[i - 1], "--ignore");
        }
    }

    #[test]
    fn light_fixes_and_validator_arguments() {
        let c = light_fixes_command(&tools(), Path::new("/c"), Path::new("/out"));
        assert_eq!(
            strings(&c),
            [
                "--openmw-cfg",
                "/c",
                "--output",
                "/out",
                "--no-notifications"
            ]
        );
        let v = validator_command(&tools(), Path::new("/c"));
        assert_eq!(strings(&v), ["--cfg", "/c", "--exit-nonzero", "--stdout"]);
    }

    #[test]
    fn navmeshtool_and_importer_arguments() {
        let c = navmeshtool_command(
            Path::new("/i/openmw/bin/openmw-navmeshtool"),
            Path::new("/profile"),
            Path::new("/profile"),
            8,
        );
        assert_eq!(
            strings(&c),
            [
                "--remove-unused-tiles",
                "--threads",
                "8",
                "--replace=config",
                "--config",
                "/profile",
                "--user-data",
                "/profile"
            ]
        );
        let i = iniimporter_command(
            Path::new("/i/openmw/bin/openmw-iniimporter"),
            Path::new("/g/Morrowind.ini"),
            Path::new("/tmp/out.cfg"),
        );
        assert_eq!(
            strings(&i),
            ["--ini", "/g/Morrowind.ini", "--cfg", "/tmp/out.cfg"]
        );
    }

    #[test]
    fn tes3cmd_defaults_to_clean_and_anchors_backups() {
        let plain = tes3cmd_command(
            &tools(),
            Path::new("/s/X.ESP"),
            &[],
            Path::new("/bk"),
            Path::new("/g/Data Files"),
        );
        assert_eq!(strings(&plain), ["clean", "/s/X.ESP"]);
        assert_eq!(plain.cwd, Some(PathBuf::from("/g/Data Files")));

        let args: Vec<String> = ["delete", "--backup-dir", "tes3cmdbck", "--hide-backups"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let delete = tes3cmd_command(
            &tools(),
            Path::new("/s/X.esp"),
            &args,
            Path::new("/bk"),
            Path::new("/g"),
        );
        assert_eq!(
            strings(&delete),
            [
                "delete",
                "--backup-dir",
                "/bk/tes3cmdbck",
                "--hide-backups",
                "/s/X.esp"
            ]
        );
    }

    #[test]
    fn extraction_takes_only_named_root_files() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("pack.tar.gz");
        {
            let gz = flate2::write::GzEncoder::new(
                std::fs::File::create(&archive).unwrap(),
                flate2::Compression::fast(),
            );
            let mut builder = tar::Builder::new(gz);
            for (name, body) in [
                ("delta_plugin", "#!/bin/sh\n"),
                ("version.txt", "Delta Plugin version: 0.25.3\n"),
                ("umo", "not wanted"),
                ("Readmes/delta_plugin", "nested, ignored"),
            ] {
                let mut header = tar::Header::new_gnu();
                header.set_size(body.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder
                    .append_data(&mut header, name, body.as_bytes())
                    .unwrap();
            }
            builder
                .into_inner()
                .unwrap()
                .finish()
                .unwrap()
                .flush()
                .unwrap();
        }
        let out = dir.path().join("out");
        std::fs::create_dir(&out).unwrap();
        let wanted = vec![
            ("delta_plugin".to_owned(), "delta_plugin"),
            ("version.txt".to_owned(), "version.txt"),
        ];
        extract_wanted(&archive, &out, &wanted).unwrap();
        assert_eq!(
            std::fs::read_to_string(out.join("delta_plugin")).unwrap(),
            "#!/bin/sh\n"
        );
        assert!(out.join("version.txt").is_file());
        assert!(!out.join("umo").exists());
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(out.join("delta_plugin"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111);
    }

    #[tokio::test]
    async fn run_reports_failure_with_the_tools_last_output() {
        let command = ToolCommand::new(
            "test tool",
            PathBuf::from("/bin/sh"),
            vec![os("-c"), os("echo first; echo boom >&2; exit 3")],
        );
        let seen = std::sync::Mutex::new(Vec::new());
        let err = run(&command, &CancellationToken::new(), &|l| {
            seen.lock().unwrap().push(l.to_owned())
        })
        .await
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("test tool failed") && err.contains("boom"),
            "{err}"
        );
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn run_can_be_cancelled() {
        let command = ToolCommand::new(
            "slow tool",
            PathBuf::from("/bin/sh"),
            vec![os("-c"), os("sleep 30")],
        );
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            trigger.cancel();
        });
        let err = run(&command, &cancel, &|_| {})
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("cancelled"), "{err}");
    }
}
