//! OpenMW tab: browse modding-openmw.com lists and review what installing one
//! would do. Network I/O runs on a worker thread; the view only renders.
//! Installing is not wired up yet, and the tab says so.
use eframe::egui;
use modyssey::momw::{
    build_plan,
    plan::{DataEntryKind, MomwPlan},
    source::{is_advertised, MomwClient, SourceOptions},
};
use std::sync::mpsc::{channel, Receiver, Sender};

enum Message {
    Lists(Result<Vec<String>, String>),
    Plan(String, Result<Box<MomwPlan>, String>),
}

#[derive(Default)]
pub struct MomwView {
    slugs: Vec<String>,
    selected: String,
    receiver: Option<Receiver<Message>>,
    started: bool,
    busy: bool,
    status: String,
    error: Option<String>,
    plan: Option<Box<MomwPlan>>,
    filter: String,
    show_all: bool,
}

fn title(slug: &str) -> String {
    slug.replace('-', " ")
}

/// Run `work` on a short-lived thread with its own runtime, then wake the UI.
fn spawn<F, Fut>(ctx: &egui::Context, sender: Sender<Message>, work: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Message>,
{
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let message = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt.block_on(work()),
            Err(e) => Message::Lists(Err(format!("could not start runtime: {e}"))),
        };
        let _ = sender.send(message);
        ctx.request_repaint();
    });
}

impl MomwView {
    fn request_lists(&mut self, ctx: &egui::Context) {
        let (tx, rx) = channel();
        self.receiver = Some(rx);
        self.busy = true;
        self.error = None;
        self.status = "Loading the list of OpenMW modlists…".into();
        spawn(ctx, tx, || async {
            let result = async {
                MomwClient::new(SourceOptions::default())?
                    .list_slugs()
                    .await
            }
            .await;
            Message::Lists(result.map_err(|e| format!("{e:#}")))
        });
    }

    fn request_plan(&mut self, ctx: &egui::Context) {
        if self.selected.is_empty() {
            return;
        }
        let slug = self.selected.clone();
        let (tx, rx) = channel();
        self.receiver = Some(rx);
        self.busy = true;
        self.error = None;
        self.plan = None;
        self.status = format!("Reviewing {}…", title(&slug));
        spawn(ctx, tx, move || async move {
            let result = async {
                let data = MomwClient::new(SourceOptions::default())?
                    .fetch_list(&slug)
                    .await?;
                Ok::<_, anyhow::Error>(Box::new(build_plan(&data)))
            }
            .await;
            Message::Plan(slug, result.map_err(|e| format!("{e:#}")))
        });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.receiver else { return };
        let Ok(message) = rx.try_recv() else { return };
        self.receiver = None;
        self.busy = false;
        self.status.clear();
        match message {
            Message::Lists(Ok(slugs)) => {
                if self.selected.is_empty() || !slugs.contains(&self.selected) {
                    self.selected = slugs
                        .iter()
                        .find(|s| *s == "total-overhaul")
                        .or(slugs.first())
                        .cloned()
                        .unwrap_or_default();
                }
                self.slugs = slugs;
                self.request_plan(ctx);
            }
            Message::Plan(slug, Ok(plan)) => {
                // Ignore a late answer for a list the user already left.
                if slug == self.selected {
                    self.plan = Some(plan);
                }
            }
            Message::Lists(Err(e)) | Message::Plan(_, Err(e)) => self.error = Some(e),
        }
    }

    pub fn render(&mut self, ctx: &egui::Context) {
        if !self.started {
            self.started = true;
            self.request_lists(ctx);
        }
        self.poll(ctx);

        if let Some(plan) = &self.plan {
            egui::SidePanel::right("momw_notes")
                .default_width(380.0)
                .show(ctx, |ui| notes(ui, plan));
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("OpenMW modlists");
            ui.label(
                "Lists published by modding-openmw.com. Review shows the mods, load order \
                 and tools a list needs, using only public site data.",
            );
            ui.add_space(6.0);
            let mut review = false;
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!self.busy && !self.slugs.is_empty(), |ui| {
                    let before = self.selected.clone();
                    egui::ComboBox::from_id_salt("momw_list")
                        .width(280.0)
                        .selected_text(title(&self.selected))
                        .show_ui(ui, |ui| {
                            for slug in &self.slugs {
                                // Keep the open list visible even if it is hidden by default.
                                if self.show_all || is_advertised(slug) || *slug == self.selected {
                                    ui.selectable_value(
                                        &mut self.selected,
                                        slug.clone(),
                                        title(slug),
                                    );
                                }
                            }
                        });
                    review |= before != self.selected;
                    review |= ui.button("Review").clicked();
                });
                ui.checkbox(&mut self.show_all, "Show WIP / unlisted")
                    .on_hover_text(
                        "Work-in-progress and retired lists the website does not advertise.",
                    );
                if ui
                    .add_enabled(!self.busy, egui::Button::new("Refresh lists"))
                    .clicked()
                {
                    self.plan = None;
                    self.request_lists(ctx);
                }
                if self.busy {
                    ui.spinner();
                    ui.label(&self.status);
                }
            });
            if review {
                self.request_plan(ctx);
            }
            if let Some(error) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_enabled(false, egui::Button::new("Install"))
                    .on_disabled_hover_text("Installing OpenMW lists is not available yet.");
                ui.weak("Installing is not available yet. This tab is a preview.");
            });
            ui.separator();

            let Some(plan) = &self.plan else { return };
            let s = &plan.summary;
            let sources: Vec<String> = s
                .sources
                .iter()
                .map(|(k, n)| format!("{} {n}", k.replace('_', " ")))
                .collect();
            ui.label(format!(
                "{} mods · {} archives ({}) · {} Fluorine mods ({} nested) · {} plugins · {} groundcover",
                s.mods,
                s.archives,
                sources.join(", "),
                s.data_entries,
                s.nested_data_entries,
                s.content_files,
                s.groundcover_files,
            ));
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("Filter mods…")
                    .desired_width(260.0),
            );
            ui.add_space(4.0);
            entries_table(ui, plan, &self.filter);
        });
    }
}

fn notes(ui: &mut egui::Ui, plan: &MomwPlan) {
    egui::ScrollArea::vertical()
        .id_salt("momw_notes_scroll")
        .show(ui, |ui| {
            ui.heading("Requirements");
            if plan.tools.is_empty() {
                ui.label("No post-install tools needed.");
            } else {
                let tools: Vec<String> = plan.tools.iter().map(|t| format!("{t:?}")).collect();
                ui.label(format!("Tools needed: {}", tools.join(", ")));
            }
            if !plan.generated_content.is_empty() {
                ui.label(format!(
                    "Created by those tools: {}",
                    plan.generated_content.join(", ")
                ));
            }
            ui.add_space(8.0);
            ui.heading("Blockers");
            if plan.blockers.is_empty() {
                ui.label("No blockers found.");
            }
            for b in &plan.blockers {
                ui.colored_label(ui.visuals().error_fg_color, b);
            }
            if !plan.warnings.is_empty() {
                ui.add_space(8.0);
                ui.heading("Warnings");
                for w in &plan.warnings {
                    ui.colored_label(ui.visuals().warn_fg_color, w);
                }
            }
        });
}

fn entries_table(ui: &mut egui::Ui, plan: &MomwPlan, filter: &str) {
    let needle = filter.trim().to_lowercase();
    let rows: Vec<_> = plan
        .data_entries
        .iter()
        .filter(|d| needle.is_empty() || d.instance_name.to_lowercase().contains(&needle))
        .collect();
    let row_height = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
    egui::ScrollArea::vertical()
        .id_salt("momw_entries")
        .auto_shrink([false, false])
        .show_rows(ui, row_height, rows.len(), |ui, range| {
            for d in &rows[range] {
                let folder = match &d.kind {
                    DataEntryKind::GameData => "Morrowind Data Files (game)".to_owned(),
                    DataEntryKind::Mod {
                        category,
                        data_path,
                        ..
                    } => format!("{category}/{data_path}"),
                    DataEntryKind::Unmapped { raw } => raw.clone(),
                };
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [48.0, row_height],
                        egui::Label::new(format!("{}", d.order + 1)),
                    );
                    let name = ui.label(&d.instance_name);
                    if d.nested_in.is_some() {
                        name.on_hover_text("Inside another data folder; split out during install.");
                    }
                    ui.weak(folder);
                });
            }
        });
}
