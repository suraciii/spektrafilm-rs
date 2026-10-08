use spektrafilm_core::params::couplers::DirCouplersParams;

pub fn show(ui: &mut egui::Ui, d: &mut DirCouplersParams) -> bool {
    let mut changed = false;
    egui::CollapsingHeader::new("DIR couplers")
        .default_open(true)
        .show(ui, |ui| {
            changed |= ui.checkbox(&mut d.active, "Active").changed();
            changed |= ui.add(egui::Slider::new(&mut d.amount, 0.0..=2.0).text("Amount")).changed();
            changed |= ui
                .add(egui::Slider::new(&mut d.diffusion_size_um, 0.0..=100.0).text("Diffusion size (µm)"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut d.diffusion_tail_um, 0.0..=400.0).text("Diffusion tail (µm)"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut d.diffusion_tail_weight, 0.0..=1.0).text("Tail weight"))
                .changed();
        });
    changed
}
