//! The right column: the plot and the numbers.
//!
//! The graph itself is a later milestone. This milestone gives the numbers table a
//! real body, so a run has somewhere to show its result. The page never shows an empty
//! gap.

use crate::theme::Tokens;
use crate::widgets::{empty_state, mono, sans, section_header};
use egui::Ui;
use std::collections::HashMap;
use vqa_core::metric::MetricId;
use vqa_core::pooling::Pooled;
use vqa_core::set::FileId;
use vqa_run::Session;

pub fn show(
    ui: &mut Ui,
    tokens: &Tokens,
    session: &Session,
    results: &HashMap<(FileId, MetricId), Pooled>,
) {
    section_header(ui, tokens, "3", "Plot");
    empty_state(ui, tokens, "The graph arrives in a later milestone.", 260.0);

    ui.add_space(16.0);
    ui.label(sans("Results", 15.0, tokens.text));
    ui.add_space(6.0);

    if results.is_empty() {
        empty_state(ui, tokens, "The numbers appear here.", 120.0);
        return;
    }

    numbers_table(ui, tokens, session, results);
}

fn numbers_table(
    ui: &mut Ui,
    tokens: &Tokens,
    session: &Session,
    results: &HashMap<(FileId, MetricId), Pooled>,
) {
    egui::Frame::default()
        .fill(tokens.sunken)
        .stroke(egui::Stroke::new(1.0, tokens.border))
        .corner_radius(crate::theme::RADIUS)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::both().max_height(246.0).show(ui, |ui| {
                header_row(ui, tokens);
                for encode in session.files.encodes() {
                    for metric in vqa_core::metric::REGISTRY.iter().map(|def| def.id) {
                        let Some(pooled) = results.get(&(encode.id, metric)) else {
                            continue;
                        };
                        data_row(ui, tokens, &encode.label, metric, pooled);
                    }
                }
            });
        });
}

fn header_row(ui: &mut Ui, tokens: &Tokens) {
    ui.horizontal(|ui| {
        for label in [
            "Video", "metric", "mean", "median", "Worst 5%", "Worst 1%", "min", "max", "std dev",
        ] {
            ui.add_sized(
                [80.0, 16.0],
                egui::Label::new(mono(label, 10.5, tokens.text_muted)),
            );
        }
    });
}

fn data_row(ui: &mut Ui, tokens: &Tokens, encode_label: &str, metric: MetricId, pooled: &Pooled) {
    let def = metric.def();
    let bad_end = def.direction.bad_end();
    let worst_5 = match bad_end {
        vqa_core::metric::Percentile::P5 => pooled.p5,
        vqa_core::metric::Percentile::P95 => pooled.p95,
    };
    let worst_1 = match bad_end {
        vqa_core::metric::Percentile::P5 => pooled.p1,
        vqa_core::metric::Percentile::P95 => pooled.p99,
    };

    ui.horizontal(|ui| {
        let cell = |ui: &mut Ui, text: String| {
            ui.add_sized(
                [80.0, 16.0],
                egui::Label::new(mono(text, 11.0, tokens.text)),
            );
        };
        cell(ui, encode_label.to_string());
        cell(ui, def.label.to_string());
        cell(ui, format!("{:.2}", pooled.mean));
        cell(ui, format!("{:.2}", pooled.median));
        cell(ui, format!("{worst_5:.2}"));
        cell(ui, format!("{worst_1:.2}"));
        cell(ui, format!("{:.2}", pooled.min));
        cell(ui, format!("{:.2}", pooled.max));
        cell(ui, format!("{:.2}", pooled.stdev));
    });
}
