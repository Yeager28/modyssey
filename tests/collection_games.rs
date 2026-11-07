//! Cross-game conformance uses small synthetic payloads, never user installs.
use clf3::collection::{
    games::{self, GameProfile},
    package::digest_file,
    publish::{publish, PublicationOptions},
    stage::stage,
    worker::{execute, recover, WorkerRequest},
    CollectionPackage, CollectionPlan, PlanOptions,
};
use clf3::collection_app::{atomic_json, Inputs};
use serde_json::json;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn plugin(game: &GameProfile, master: Option<&str>, flags: u32) -> Vec<u8> {
    let mut body = b"HEDR\x0c\0".to_vec();
    body.extend_from_slice(&1.0f32.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&0x800u32.to_le_bytes());
    if let Some(master) = master {
        body.extend_from_slice(b"MAST");
        body.extend_from_slice(&((master.len() + 1) as u16).to_le_bytes());
        body.extend_from_slice(master.as_bytes());
        body.push(0);
        body.extend_from_slice(b"DATA\x08\0");
        body.extend_from_slice(&[0; 8]);
    }
    let mut data = vec![0; game.plugin_header_size];
    data[..4].copy_from_slice(b"TES4");
    data[4..8].copy_from_slice(&(body.len() as u32).to_le_bytes());
    data[8..12].copy_from_slice(&flags.to_le_bytes());
    data.extend(body);
    data
}

fn fixture(root: &Path, game: &GameProfile) -> (CollectionPackage, CollectionPlan) {
    let dir = root.join("package");
    write(&dir.join("collection.json"), serde_json::to_vec(&json!({
        "info":{"domainName":game.nexus_domain(),"name":"Cross-game fixture","gameVersions":game.variant.as_ref().map(|v| vec![format!("1.0.0.0{}", v.version_suffix.to_uppercase())]).unwrap_or_default()},
        "mods":[{"name":"Exact payload","source":{"type":"bundle","fileExpression":"A","tag":"a"}}],
        "plugins":[{"name":"A.esp","enabled":true},{"name":"B.esp","enabled":true},{"name":"Off.esp","enabled":false}]
    })).unwrap());
    write(
        &dir.join(format!("bundled/A/{}", game.extender)),
        b"fixture extender",
    );
    write(
        &dir.join(format!(
            "bundled/A/Data/{}/Plugins/Keep.dll",
            game.extender_directory
        )),
        b"extender plugin",
    );
    write(
        &dir.join("bundled/A/Data/A.esp"),
        plugin(game, Some("B.esp"), 0),
    );
    write(
        &dir.join("bundled/A/Data/B.esp"),
        plugin(game, Some(game.base_plugins[0]), 0),
    );
    write(
        &dir.join("bundled/A/Data/Off.esp"),
        plugin(game, Some("MissingOptional.esm"), 0),
    );
    let stem = game.ini_files[0].trim_end_matches(".ini");
    write(
        &dir.join(format!("INI Tweaks/Profile [{stem}].ini")),
        b"[General]\nbAlwaysActive=1\n",
    );
    // Variant games declare game versions, so their executable carries a
    // version resource (1.0.0.0) for the planner to read back.
    let mut exe = b"fixture game executable".to_vec();
    if game.variant.is_some() {
        exe.extend([0xbd, 0x04, 0xef, 0xfe, 0, 0, 1, 0]);
        exe.extend(1u32.wrapping_shl(16).to_le_bytes());
        exe.extend(0u32.to_le_bytes());
    }
    write(&root.join("game").join(game.executable), exe);
    write(
        &root.join("game").join(game.default_ini),
        b"[General]\noriginal=1\n",
    );
    write(
        &root.join("game/Data").join(game.base_plugins[0]),
        plugin(game, None, 1),
    );
    let package = CollectionPackage::open(&dir).unwrap();
    let plan = CollectionPlan::build(
        &package,
        &PlanOptions {
            schema_id: Some(1),
            game_version: game.variant.as_ref().map(|_| "1.0.0.0".to_owned()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        plan.blockers.is_empty(),
        "{}: {:?}",
        game.domain,
        plan.blockers
    );
    // The collection must resolve to the adapter under test, not its base game.
    assert_eq!(plan.profile().unwrap().domain, game.domain);
    (package, plan)
}

#[test]
fn each_game_publishes_its_own_layout_ini_launcher_and_activation_with_and_without_loot() {
    for game in games::PROFILES {
        for with_loot in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            let (package, plan) = fixture(root, game);
            let source = root.join("game/Data").join(game.base_plugins[0]);
            let source_hash = digest_file(&source).unwrap();
            let source_time = source.metadata().unwrap().modified().unwrap();
            let staged = stage(
                &package,
                &plan,
                &BTreeMap::new(),
                &root.join("stage"),
                &Default::default(),
            )
            .unwrap();
            let member = staged.members.values().next().unwrap();
            assert!(member
                .files
                .iter()
                .any(|f| f.staged_path == format!("Root/{}", game.extender)
                    && f.deployment_root == "game"));
            assert!(member.files.iter().any(|f| f.staged_path
                == format!("{}/Plugins/Keep.dll", game.extender_directory)
                && f.deployment_root == "data"));
            let masterlist = root.join("masterlist.yaml");
            write(&masterlist, b"plugins: []\n");
            let masterlist = std::env::var_os("CLF3_GAME_MASTERLIST_DIR")
                .map(|p| PathBuf::from(p).join(game.domain).join("masterlist.yaml"))
                .filter(|p| p.is_file())
                .unwrap_or(masterlist);
            let output = root.join("installed");
            let report = publish(
                &root.join("stage"),
                &root.join("game"),
                &output,
                &PublicationOptions {
                    masterlist: with_loot.then_some(masterlist.as_path()),
                    ..Default::default()
                },
                &Default::default(),
            )
            .unwrap_or_else(|e| panic!("{} LOOT={with_loot}: {e:#}", game.domain));
            assert_eq!(report.enabled_plugins, 3);
            assert_eq!(report.profile_ini_edits, 1);
            assert_eq!(report.verified_mod_files, 5);
            assert_eq!(digest_file(&source).unwrap(), source_hash);
            assert_eq!(source.metadata().unwrap().modified().unwrap(), source_time);
            let config = std::fs::read_to_string(output.join("ModOrganizer.ini")).unwrap();
            assert!(config.contains(&format!("gameName={}\n", game.manager_name())));
            assert!(config.contains(&format!("/Stock Game/{}\n", game.extender)));
            // No game inherits Skyrim SE's launcher unless it is its own.
            assert!(!config.contains("SkyrimSE.exe"));
            if game.extender != "skse64_loader.exe" {
                assert!(!config.contains("skse64_loader.exe"));
            }
            let profile = output.join("profiles/Default");
            let ini = std::fs::read_to_string(profile.join(game.ini_files[0])).unwrap();
            assert!(ini.contains("bAlwaysActive=1"));
            let activation = std::fs::read_to_string(profile.join("plugins.txt")).unwrap();
            if game.asterisk_plugins {
                assert!(activation.contains("*A.esp"));
                assert!(activation.contains("Off.esp"));
            } else {
                assert!(activation.contains("\nA.esp\n"));
                assert!(!activation.contains('*'));
                assert!(!activation.contains("Off.esp"));
            }
            let order = std::fs::read_to_string(profile.join("loadorder.txt")).unwrap();
            assert!(order.find(game.base_plugins[0]).unwrap() < order.find("B.esp").unwrap());
            assert!(order.find("B.esp").unwrap() < order.find("A.esp").unwrap());
            if game.timestamp_order {
                let times: BTreeMap<String, u64> = serde_json::from_slice(
                    &std::fs::read(output.join(".collection/plugin-timestamps.json")).unwrap(),
                )
                .unwrap();
                let a = times.iter().find(|(p, _)| p.ends_with("/A.esp")).unwrap().1;
                let b = times.iter().find(|(p, _)| p.ends_with("/B.esp")).unwrap().1;
                assert!(a > b);
                for (path, seconds) in times {
                    assert_eq!(
                        output
                            .join(path)
                            .metadata()
                            .unwrap()
                            .modified()
                            .unwrap()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs(),
                        seconds
                    );
                }
            }
            if with_loot {
                let result = std::process::Command::new("python3")
                    .arg(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/scripts/collection_verify.py"
                    ))
                    .arg(&output)
                    .arg("--output")
                    .arg(root.join("audit.json"))
                    .output()
                    .unwrap();
                assert!(
                    result.status.success(),
                    "{} independent audit: {} {}",
                    game.domain,
                    String::from_utf8_lossy(&result.stdout),
                    String::from_utf8_lossy(&result.stderr)
                );
            }
        }
    }
}

#[test]
fn wrong_game_and_cross_game_ini_are_refused_before_installation() {
    let temp = tempfile::tempdir().unwrap();
    let game = games::require("fallout4").unwrap();
    let (_, _) = fixture(temp.path(), game);
    let inputs = Inputs {
        source: temp.path().join("package").to_string_lossy().into_owned(),
        game: temp.path().join("wrong-game"),
        cache: temp.path().join("cache"),
        ..Default::default()
    };
    write(&inputs.game.join("SkyrimSE.exe"), b"wrong game");
    let error = clf3::collection_app::prepare(inputs, None, "", &Default::default())
        .err()
        .expect("wrong source game must fail");
    assert!(format!("{error:#}").contains("Fallout4.exe"));
    write(
        &temp.path().join("package/INI Tweaks/Wrong [Skyrim].ini"),
        b"[General]\nx=1\n",
    );
    let package = CollectionPackage::open(&temp.path().join("package")).unwrap();
    let plan = CollectionPlan::build(&package, &PlanOptions::default()).unwrap();
    assert!(plan
        .blockers
        .iter()
        .any(|b| b.code == "unsupported_ini_tweak"));
}

#[test]
fn classic_games_reject_light_plugins_and_encode_activation_without_stars() {
    for domain in ["newvegas", "fallout3", "oblivion", "skyrim"] {
        let game = games::require(domain).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let (package, plan) = fixture(temp.path(), game);
        // Change the bundle before opening/replanning; no plan mutation bypass.
        drop(package);
        drop(plan);
        write(
            &temp.path().join("package/bundled/A/Data/A.esp"),
            plugin(game, Some("B.esp"), 0x200),
        );
        let package = CollectionPackage::open(&temp.path().join("package")).unwrap();
        let plan = CollectionPlan::build(&package, &PlanOptions::default()).unwrap();
        stage(
            &package,
            &plan,
            &BTreeMap::new(),
            &temp.path().join("stage"),
            &Default::default(),
        )
        .unwrap();
        let error = publish(
            &temp.path().join("stage"),
            &temp.path().join("game"),
            &temp.path().join("installed"),
            &Default::default(),
            &Default::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("does not support light plugins"));
        assert!(!temp.path().join("installed").exists());
        assert_eq!(
            game.plugin_list([("Café.esp", true), ("Off.esp", false)])
                .unwrap(),
            b"Caf\xe9.esp\n"
        );
    }
}

#[test]
fn worker_recovery_checks_each_games_ini_and_timestamp_state() {
    for domain in ["fallout4", "newvegas"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let game = games::require(domain).unwrap();
        let (_, plan) = fixture(root, game);
        atomic_json(&root.join("plan.json"), &plan).unwrap();
        atomic_json(
            &root.join("artifacts.json"),
            &BTreeMap::<String, PathBuf>::new(),
        )
        .unwrap();
        let request = WorkerRequest {
            protocol_version: 1,
            job_identity: Some("game-fixture".into()),
            package: root.join("package"),
            plan: root.join("plan.json"),
            artifacts: root.join("artifacts.json"),
            stage: root.join("stage"),
            game: root.join("game"),
            output: root.join("installed"),
            profile_ini: None,
            masterlist: None,
            masterlist_sha256: None,
        };
        execute(&request, &Default::default(), &|_| {}).unwrap();
        recover(
            &request.output,
            "game-fixture",
            &plan,
            &Default::default(),
            &|_| {},
        )
        .unwrap();
        if game.timestamp_order {
            let manifest = request.output.join(".collection/plugin-timestamps.json");
            let original = std::fs::read(&manifest).unwrap();
            write(&manifest, b"{}");
            assert!(recover(
                &request.output,
                "game-fixture",
                &plan,
                &Default::default(),
                &|_| {}
            )
            .unwrap_err()
            .to_string()
            .contains("timestamps"));
            write(&manifest, &original);
            let path = request
                .output
                .join("Stock Game/Data")
                .join(game.base_plugins[0]);
            std::fs::File::open(path)
                .unwrap()
                .set_modified(std::time::SystemTime::now())
                .unwrap();
            assert!(recover(
                &request.output,
                "game-fixture",
                &plan,
                &Default::default(),
                &|_| {}
            )
            .unwrap_err()
            .to_string()
            .contains("timestamps"));
        } else {
            write(
                &request
                    .output
                    .join("profiles/Default")
                    .join(game.ini_files[0]),
                b"changed",
            );
            assert!(recover(
                &request.output,
                "game-fixture",
                &plan,
                &Default::default(),
                &|_| {}
            )
            .unwrap_err()
            .to_string()
            .contains("profile changed"));
        }
    }
}

#[test]
fn unimplemented_adapters_remain_blocked_with_game_specific_reasons() {
    for domain in [
        "cyberpunk2077",
        "stardewvalley",
        "baldursgate3",
        "morrowind",
        "starfield",
        "oblivionremastered",
    ] {
        let temp = tempfile::tempdir().unwrap();
        write(
            &temp.path().join("collection.json"),
            serde_json::to_vec(
                &json!({"info":{"domainName":domain,"name":"Unsupported"},"mods":[]}),
            )
            .unwrap(),
        );
        let package = CollectionPackage::open(temp.path()).unwrap();
        let plan = CollectionPlan::build(&package, &PlanOptions::default()).unwrap();
        assert!(plan.blockers.iter().any(
            |b| b.code == "unsupported_game" && b.message == games::unsupported_reason(domain)
        ));
        assert!(stage(
            &package,
            &plan,
            &BTreeMap::new(),
            &temp.path().join("stage"),
            &Default::default()
        )
        .is_err());
    }
}

#[test]
fn hosted_installation_cancels_resumes_and_verifies_every_game_adapter() {
    use clf3::collection::host::install_local;
    for game in games::PROFILES {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let (_, plan) = fixture(root, game);
        let job = root.join("host-job");
        atomic_json(&job.join("plan.json"), &plan).unwrap();
        atomic_json(
            &job.join("artifacts.json"),
            &BTreeMap::<String, PathBuf>::new(),
        )
        .unwrap();
        let mut request = WorkerRequest {
            protocol_version: 1,
            job_identity: Some("hosted-game-fixture".into()),
            package: root.join("package"),
            plan: job.join("plan.json"),
            artifacts: job.join("artifacts.json"),
            stage: job.join("stage"),
            game: root.join("game"),
            output: root.join("installed"),
            profile_ini: None,
            masterlist: None,
            masterlist_sha256: None,
        };
        let original = digest_file(&request.game.join(game.executable)).unwrap();
        let token = tokio_util::sync::CancellationToken::new();
        assert!(install_local(&request, &token, &|_| token.cancel()).is_err());
        assert!(!request.output.exists());
        install_local(&request, &Default::default(), &|_| {})
            .unwrap_or_else(|e| panic!("{}: {e:#}", game.domain));
        let before = std::fs::read(request.output.join("profiles/Default/modlist.txt")).unwrap();
        install_local(&request, &Default::default(), &|_| {}).unwrap();
        assert_eq!(
            std::fs::read(request.output.join("profiles/Default/modlist.txt")).unwrap(),
            before
        );
        assert_eq!(
            digest_file(&request.game.join(game.executable)).unwrap(),
            original
        );
        let report: serde_json::Value = serde_json::from_slice(
            &std::fs::read(request.output.join(".collection/report.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(report["runtime_launch_tested"], false);
        request.job_identity = Some("different-host-job".into());
        assert!(install_local(&request, &Default::default(), &|_| {}).is_err());
        assert_eq!(
            std::fs::read(request.output.join("profiles/Default/modlist.txt")).unwrap(),
            before
        );
    }
}

#[test]
fn enderal_profiles_follow_their_engines() {
    let le = games::require("enderal").unwrap();
    let se = games::require("enderalspecialedition").unwrap();
    // Enderal runs on the Skyrim LE engine and Enderal SE on the SE engine.
    assert!(!le.asterisk_plugins && !le.light_plugins && !le.timestamp_order);
    assert!(se.asterisk_plugins && se.light_plugins && !se.timestamp_order);
    assert_eq!(le.manager_name(), "Enderal");
    assert_eq!(se.manager_name(), "Enderal Special Edition");
    assert_eq!(le.extender, "skse_loader.exe");
    assert_eq!(se.extender, "skse64_loader.exe");
    // Both launch through the one executable every install has.
    assert_eq!(le.executable, se.executable);
    // Skyrim collections are not Enderal collections, and vice versa.
    for other in ["skyrim", "skyrimspecialedition"] {
        assert_ne!(games::require(other).unwrap().my_games, le.my_games);
        assert_ne!(games::require(other).unwrap().my_games, se.my_games);
    }
    // The shared LOOT masterlist is pinned once and fetched from the same URL.
    assert_eq!(le.masterlist.url(), se.masterlist.url());
    assert!(le.masterlist.url().contains("/loot/enderal/63b9f2d1"));
    // The SE plugin list carries the two plugins the game does not load alone.
    let list = se
        .plugin_list([("Enderal - Forgotten Stories.esm", true), ("SkyUI_SE.esp", true)])
        .unwrap();
    assert_eq!(list, b"*Enderal - Forgotten Stories.esm\n*SkyUI_SE.esp\n");
    // Enderal LE lists only enabled plugins, without stars.
    let list = le.plugin_list([("A.esp", true), ("B.esp", false)]).unwrap();
    assert_eq!(list, b"A.esp\n");
}

fn versions(list: &[&str]) -> Vec<String> {
    list.iter().map(|v| (*v).to_owned()).collect()
}

#[test]
fn vr_collections_resolve_from_their_game_versions() {
    // Nexus files VR mods under the base game's domain, so only the game
    // versions name the edition.
    let vr = |d, v: &[&str]| games::resolve(d, &versions(v)).map(|g| g.domain);
    assert_eq!(vr("skyrimspecialedition", &["1.4.15.0-VR"]), Some("skyrimvr"));
    assert_eq!(vr("skyrimspecialedition", &[" 1.4.15.0-vr "]), Some("skyrimvr"));
    assert_eq!(vr("fallout4", &["1.2.72.0-VR"]), Some("fallout4vr"));
    // Base-game collections, with no or ordinary versions, stay base-game.
    assert_eq!(vr("skyrimspecialedition", &[]), Some("skyrimspecialedition"));
    assert_eq!(vr("skyrimspecialedition", &["1.6.1170.0"]), Some("skyrimspecialedition"));
    assert_eq!(vr("fallout4", &["1.10.163.0"]), Some("fallout4"));
    // A collection that also lists the base game's versions is not VR-only.
    assert_eq!(
        vr("skyrimspecialedition", &["1.6.1170.0", "1.4.15.0-VR"]),
        Some("skyrimspecialedition")
    );
    // The suffix never moves a game onto another game's VR edition.
    assert_eq!(vr("fallout4", &["1.4.15.0-VR"]), Some("fallout4vr"));
    assert_eq!(vr("newvegas", &["1.4.0.525-VR"]), Some("newvegas"));
    assert_eq!(vr("skyrim", &["1.9.32.0-VR"]), Some("skyrim"));
}

#[test]
fn vr_profiles_follow_their_engines() {
    let svr = games::require("skyrimvr").unwrap();
    let fvr = games::require("fallout4vr").unwrap();
    assert_eq!(svr.nexus_domain(), "skyrimspecialedition");
    assert_eq!(fvr.nexus_domain(), "fallout4");
    assert_eq!(svr.executable, "SkyrimVR.exe");
    assert_eq!(fvr.executable, "Fallout4VR.exe");
    assert_eq!(svr.extender, "sksevr_loader.exe");
    assert_eq!(fvr.extender, "f4sevr_loader.exe");
    // Different launcher, game folder and Steam app than the flat games.
    for (vr, flat) in [(svr, "skyrimspecialedition"), (fvr, "fallout4")] {
        let flat = games::require(flat).unwrap();
        assert_ne!(vr.executable, flat.executable);
        assert_ne!(vr.extender, flat.extender);
        assert_ne!(vr.my_games, flat.my_games);
        assert!(vr.steam_ids.iter().all(|id| !flat.steam_ids.contains(id)));
        assert!(vr.asterisk_plugins && !vr.timestamp_order);
    }
    assert_eq!(svr.manager_name(), "Skyrim VR");
    assert_eq!(fvr.manager_name(), "Fallout 4 VR");
    // Their own LOOT masterlists, not the flat game's.
    assert!(svr.masterlist.url().contains("/loot/skyrimvr/"));
    assert!(fvr.masterlist.url().contains("/loot/fallout4vr/"));
    // Flat games must never claim a variant.
    for flat in ["skyrimspecialedition", "fallout4"] {
        assert!(games::require(flat).unwrap().variant.is_none());
    }
}

#[test]
fn a_vr_collection_is_refused_on_the_flat_games_folder() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let svr = games::require("skyrimvr").unwrap();
    let (package, plan) = fixture(root, svr);
    assert_eq!(plan.domain, "skyrimspecialedition");
    assert_eq!(plan.profile().unwrap().domain, "skyrimvr");
    let staged = stage(
        &package,
        &plan,
        &BTreeMap::new(),
        &root.join("stage"),
        &Default::default(),
    )
    .unwrap();
    drop(staged);
    // A Skyrim SE folder has SkyrimSE.exe, not SkyrimVR.exe.
    let wrong = root.join("flat-game");
    write(&wrong.join("SkyrimSE.exe"), b"flat game");
    let err = clf3::collection::publish::publish(
        &root.join("stage"),
        &wrong,
        &root.join("out"),
        &Default::default(),
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        format!("{err:#}").contains("SkyrimVR.exe"),
        "unexpected error: {err:#}"
    );
    assert!(!root.join("out").exists());
}

fn blockers_for(domain: &str, declared: &[&str], runtime: &str) -> Vec<String> {
    let temp = tempfile::tempdir().unwrap();
    write(
        &temp.path().join("collection.json"),
        serde_json::to_vec(&json!({
            "info":{"domainName":domain,"name":"Versions","gameVersions":declared},
            "mods":[]
        }))
        .unwrap(),
    );
    let package = CollectionPackage::open(temp.path()).unwrap();
    CollectionPlan::build(
        &package,
        &PlanOptions {
            schema_id: Some(1),
            game_version: Some(runtime.into()),
            ..Default::default()
        },
    )
    .unwrap()
    .blockers
    .into_iter()
    .map(|b| b.code)
    .collect()
}

#[test]
fn vr_runtime_versions_match_their_suffixed_collection_versions() {
    // The VR executable reports 1.4.15.0; the collection writes 1.4.15.0-VR.
    assert!(blockers_for("skyrimspecialedition", &["1.4.15.0-VR"], "1.4.15.0").is_empty());
    assert!(blockers_for("skyrimspecialedition", &["1.4.15.0-vr"], "1.4.15.0").is_empty());
    assert!(blockers_for("fallout4", &["1.2.72.0-VR"], "1.2.72.0").is_empty());
    // A different VR build is still a mismatch.
    assert_eq!(
        blockers_for("skyrimspecialedition", &["1.4.15.0-VR"], "1.4.15.1"),
        ["game_version_mismatch"]
    );
    // Flat games stay exact: a mixed collection installs as the flat game, and a
    // VR runtime must not satisfy it through the suffix rule.
    assert_eq!(
        blockers_for("skyrimspecialedition", &["1.6.1170.0", "1.4.15.0-VR"], "1.4.15.0"),
        ["game_version_mismatch"]
    );
    assert!(blockers_for("skyrimspecialedition", &["1.6.1170.0"], "1.6.1170.0").is_empty());
    assert_eq!(
        blockers_for("skyrimspecialedition", &["1.4.15.0-VR"], "1.6.1170.0"),
        ["game_version_mismatch"]
    );
}
