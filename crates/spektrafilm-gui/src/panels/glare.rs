use spektrafilm_core::params::glare::GlareParams;

pub fn show(ui: &mut egui::Ui, g: &mut GlareParams, scan_film: bool) -> bool {
    let mut changed = false;
    egui::CollapsingHeader::new("Glare")
        .default_open(false)
        .show(ui, |ui| {
            if scan_film {
                ui.label(
                    egui::RichText::new(
                        "Direct film scan — viewing glare is disabled (print-only effect).",
                    )
                    .italics()
                    .small(),
                );
            }
            ui.add_enabled_ui(!scan_film, |ui| {
                changed |= ui.checkbox(&mut g.active, "Active").changed();
                changed |= ui
                    .add(egui::Slider::new(&mut g.percent, 0.0..=0.2).text("Percent"))
                    .changed();
                changed |= ui
                    .add(egui::Slider::new(&mut g.roughness, 0.0..=2.0).text("Roughness"))
                    .changed();
                changed |= ui
                    .add(egui::Slider::new(&mut g.blur, 0.0..=5.0).text("Blur σ (px)"))
                    .changed();
            });
        });
    changed
}
