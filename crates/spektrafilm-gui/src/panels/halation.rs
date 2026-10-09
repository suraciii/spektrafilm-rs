use spektrafilm_core::params::halation::HalationParams;

pub fn show(ui: &mut egui::Ui, h: &mut HalationParams) -> bool {
    let mut changed = false;
    egui::CollapsingHeader::new("Halation")
        .default_open(true)
        .show(ui, |ui| {
            changed |= ui.checkbox(&mut h.active, "Active").changed();
            changed |= ui
                .add(egui::Slider::new(&mut h.halation_amount, 0.0..=3.0).text("Halation amount"))
                .changed();
            changed |= ui
                .add(
                    egui::Slider::new(&mut h.halation_spatial_scale, 0.1..=5.0)
                        .text("Halation scale"),
                )
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut h.scatter_amount, 0.0..=3.0).text("Scatter amount"))
                .changed();
            changed |= ui
                .add(
                    egui::Slider::new(&mut h.scatter_spatial_scale, 0.1..=5.0)
                        .text("Scatter scale"),
                )
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut h.halation_n_bounces, 1..=5).text("Bounces"))
                .changed();
            changed |= ui
                .add(
                    egui::Slider::new(&mut h.halation_bounce_decay, 0.0..=1.0).text("Bounce decay"),
                )
                .changed();
            changed |= ui
                .checkbox(&mut h.halation_renormalize, "Renormalize")
                .changed();
        });
    changed
}
