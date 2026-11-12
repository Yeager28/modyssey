//! Read-only source/package checks. No game installation or artifact download.
use ds2modpack::collection::{games, CollectionPackage, CollectionPlan, PlanOptions};
use ds2modpack::collection_app::{
    acquire::Nexus,
    catalog::{Catalog, Search},
};
use std::collections::BTreeMap;

#[test]
#[ignore = "downloads public collection packages using the saved account into a caller-chosen audit directory"]
fn real_collection_packages_and_game_masterlists_are_reviewable() {
    let root = std::path::PathBuf::from(
        std::env::var_os("DSMODPACK_GAME_AUDIT_DIR").expect("Set DSMODPACK_GAME_AUDIT_DIR"),
    );
    std::fs::create_dir_all(&root).unwrap();
    let settings = ds2modpack::settings::Settings::load();
    let nexus = Nexus::new(&settings.nexus_api_key).unwrap();
    let catalog = Catalog::new().unwrap();
    let token = Default::default();
    let mut reports = Vec::new();
    for game in games::PROFILES.iter().filter(|g| g.experimental) {
        // A VR edition has no Nexus domain of its own: search its base game for
        // collections and keep the first whose game versions resolve to it.
        let page = catalog
            .search(
                &Search {
                    game: game.nexus_domain().into(),
                    text: if game.variant.is_some() { "VR".into() } else { String::new() },
                    ..Default::default()
                },
                &token,
            )
            .unwrap();
        let candidates = page
            .nodes
            .iter()
            .filter(|e| e.can_review_installation())
            .take(if game.variant.is_some() { 10 } else { 1 });
        let mut found = None;
        for entry in candidates {
            let locator =
                ds2modpack::collection::url::parse_collection_url(&entry.url().unwrap()).unwrap();
            let directory = root.join(game.domain);
            let (path, pinned, schema) = nexus.package(&locator, &directory, &token).unwrap();
            let package = CollectionPackage::open(&path).unwrap();
            if package.collection.require_profile().unwrap().domain == game.domain {
                found = Some((directory, pinned, schema, package));
                break;
            }
        }
        let (directory, pinned, schema, package) =
            found.expect("Published schema-1 collection for this adapter");
        assert_eq!(package.collection.domain(), game.nexus_domain());
        // This is a package conformance probe, not an assertion that a locally
        // installed runtime matches. All authored compatibility checks remain.
        let version = package.collection.info.game_versions.first().cloned();
        let plan = CollectionPlan::build(
            &package,
            &PlanOptions {
                schema_id: Some(schema),
                locator: Some(pinned.clone()),
                game_version: version,
                all_optional: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!plan.blockers.iter().any(|b| b.code == "unsupported_game"));
        ds2modpack::collection_app::atomic_json(&directory.join("plan.json"), &plan).unwrap();
        ds2modpack::collection_app::acquire::masterlist_for(
            game.domain,
            &directory.join("masterlist.yaml"),
            &token,
        )
        .unwrap();
        let mut blockers = BTreeMap::<String, usize>::new();
        for blocker in &plan.blockers {
            *blockers.entry(blocker.code.clone()).or_default() += 1;
        }
        let types: std::collections::BTreeSet<_> = package
            .collection
            .mods
            .iter()
            .filter_map(|m| m.details.get("type").and_then(|v| v.as_str()))
            .collect();
        eprintln!(
            "{}: {} r{}, {} members; blockers {:?}; member types {:?}",
            game.domain,
            plan.name,
            pinned.revision.unwrap(),
            plan.members.len(),
            blockers,
            types
        );
        reports.push(serde_json::json!({"game":game.domain,"collection":plan.name,"slug":pinned.slug,"revision":pinned.revision,"members":plan.members.len(),"package_sha256":package.digest,"blockers":blockers,"member_types":types,"masterlist_sha256":game.masterlist.sha256,"game_launch_tested":false,"installed":false}));
    }
    ds2modpack::collection_app::atomic_json(&root.join("audit.json"), &reports).unwrap();
}

#[test]
#[ignore = "requires the earlier local GTS GUI validation job"]
fn existing_gts_reviewed_plan_is_unchanged_by_game_registry() {
    let path = std::path::PathBuf::from(
        std::env::var_os("DSMODPACK_GTS_GUI_JOB").expect("Set DSMODPACK_GTS_GUI_JOB"),
    );
    let prepared = ds2modpack::collection_app::Prepared::restore(&path).unwrap();
    let package = CollectionPackage::open(&prepared.package).unwrap();
    let rebuilt = CollectionPlan::build(&package, &prepared.options()).unwrap();
    assert_eq!(
        serde_json::to_value(&prepared.plan).unwrap(),
        serde_json::to_value(&rebuilt).unwrap()
    );
    eprintln!("GTS r117: existing plan, member signatures, optional choices and artifact identities unchanged");
}
