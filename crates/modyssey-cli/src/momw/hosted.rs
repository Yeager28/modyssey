//! `modyssey momw install --hosted`: the same install, driven by Fluorine.
//!
//! Stdout carries only newline-delimited JSON in the installer's existing
//! event vocabulary, so Fluorine's progress view works unchanged. Nexus
//! authorization (including the click flow for free accounts) and Nexus file
//! lists are answered by Fluorine; no credentials reach Modyssey.

use super::fetch::NexusAccess;
use super::install::Event as StageEvent;
use super::lock::InstallLock;
use super::nexus::HostLister;
use super::pipeline::{run_install, Event, EventSink, InstallRequest};
use super::plan::MomwPlan;
use crate::installer::host::StdioHost;
use crate::installer::progress_json::JsonEventWriter;
use anyhow::Result;
use serde_json::json;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

struct Progress {
    writer: JsonEventWriter,
    plan_archives: Vec<(String, String)>,
    started: Mutex<HashSet<usize>>,
    done: Mutex<usize>,
}

impl Progress {
    fn item_id(index: usize) -> String {
        format!("mod-{index}")
    }

    fn start(&self, index: usize, file: &str, total: u64, unit: &str) {
        if !self.started.lock().unwrap().insert(index) {
            return;
        }
        let (mod_name, category) = &self.plan_archives[index];
        self.writer.emit_value(&json!({
            "type": "item_started",
            "item_id": Self::item_id(index),
            "name": file,
            "display_name": mod_name,
            "subtitle": category,
            "stage": "download",
            "image_url": null,
            "total": total,
            "unit": unit,
        }));
    }

    fn handle(&self, event: Event) {
        match event {
            Event::Phase(phase) => self.writer.emit_value(&json!({
                "type": "phase_changed",
                "phase": phase.label(),
            })),
            Event::Stage(StageEvent::Downloading {
                archive,
                name,
                done,
                total,
            }) => {
                self.start(archive, &name, total, "bytes");
                self.writer.emit_value(&json!({
                    "type": "item_progress",
                    "item_id": Self::item_id(archive),
                    "completed": done,
                    "total": total,
                    "speed": 0,
                    "unit": "bytes",
                }));
            }
            Event::Stage(StageEvent::Extracting { archive, name }) => {
                self.start(archive, &name, 0, "items");
                self.writer.emit_value(&json!({
                    "type": "item_message",
                    "item_id": Self::item_id(archive),
                    "message": "Unpacking",
                }));
            }
            Event::Stage(StageEvent::Staged { archive }) => {
                // An archive already in the cache never reports a download.
                let name = self.plan_archives[archive].0.clone();
                self.start(archive, &name, 0, "items");
                self.writer.emit_value(&json!({
                    "type": "item_completed",
                    "item_id": Self::item_id(archive),
                }));
                let mut done = self.done.lock().unwrap();
                *done += 1;
                self.writer.emit_value(&json!({
                    "type": "overall_progress",
                    "index": *done,
                    "total": self.plan_archives.len(),
                }));
            }
            Event::Stage(StageEvent::Warning(message)) | Event::Note(message) => {
                self.writer
                    .emit_value(&json!({"type": "status", "message": message}));
            }
            // Tool output is verbose; it belongs in the log, not the status line.
            Event::Tool(line) => eprintln!("{line}"),
        }
    }
}

/// Everything an install needs, prepared after the handshake so that a
/// problem (no navmeshtool, another install running, no network) reaches
/// Fluorine as an `install_failed` message.
pub struct Prepared {
    pub plan: MomwPlan,
    pub request: InstallRequest,
    pub lock: InstallLock,
}

/// Run the install under Fluorine's control. Returns after emitting either
/// `install_completed` or `install_failed`.
pub async fn run_hosted<F>(prepare: F) -> Result<()>
where
    F: std::future::Future<Output = Result<Prepared>>,
{
    let writer = JsonEventWriter::stdout();
    let host = StdioHost::start(writer.clone());
    host.wait_until_ready().await?;
    let prepared = match prepare.await {
        Ok(prepared) => prepared,
        Err(error) => {
            host.emit_install_failed(&json!({}), &format!("{error:#}"));
            return Err(error);
        }
    };
    let Prepared {
        plan,
        request,
        lock: _lock,
    } = prepared;
    let (plan, request) = (&plan, &request);
    let cancel = crate::installer::host::HostedDownloadProvider::cancellation_token(&*host);

    let progress = Arc::new(Progress {
        writer,
        plan_archives: plan
            .archives
            .iter()
            .map(|a| (a.mod_name.clone(), a.category.clone()))
            .collect(),
        started: Mutex::new(HashSet::new()),
        done: Mutex::new(0),
    });
    let sink: EventSink = {
        let progress = progress.clone();
        Arc::new(move |event| progress.handle(event))
    };
    let lister = HostLister(host.clone());
    let result = run_install(
        plan,
        request,
        NexusAccess::Hosted(host.clone()),
        &lister,
        cancel,
        sink,
    )
    .await;

    match result {
        Ok(report) => {
            let stats = json!({
                "mods": report.instance.mods,
                "archives": plan.archives.len(),
                "plugins_enabled": report.instance.plugins_enabled,
                "plugins_left_off": report.instance.plugins_left_off,
                "warnings": report.warnings,
                "validator_problems": report.validator_problems,
                "instance_dir": request.root,
            });
            host.emit_install_completed(&stats, &request.game_dir);
            Ok(())
        }
        Err(error) => {
            host.emit_install_failed(&json!({}), &format!("{error:#}"));
            Err(error)
        }
    }
}
