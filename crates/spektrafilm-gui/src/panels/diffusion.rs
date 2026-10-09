use spektrafilm_core::params::diffusion::DiffusionFilterParams;

pub fn show(ui: &mut egui::Ui, df: &mut DiffusionFilterParams) -> bool {
    let mut changed = false;
    egui::CollapsingHeader::new("Diffusion filter (lens)")
        .default_open(true)
        .show(ui, |ui| {
            changed |= ui.checkbox(&mut df.active, "Active").changed();
            egui::ComboBox::from_label("Family")
                .selected_text(df.filter_family.clone())
                .show_ui(ui, |ui| {
                    for fam in ["black_pro_mist", "glimmerglass", "pro_mist", "cinebloom"] {
                        changed |= ui
                            .selectable_value(&mut df.filter_family, fam.to_string(), fam)
                            .changed();
                    }
                });
            changed |= ui
                .add(egui::Slider::new(&mut df.strength, 0.0..=2.0).text("Strength"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut df.spatial_scale, 0.1..=3.0).text("Spatial scale"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut df.halo_warmth, -1.5..=1.5).text("Halo warmth"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut df.core_intensity, 0.0..=2.0).text("Core intensity"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut df.halo_intensity, 0.0..=2.0).text("Halo intensity"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut df.bloom_intensity, 0.0..=2.0).text("Bloom intensity"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut df.halo_size, 0.1..=3.0).text("Halo size"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut df.bloom_size, 0.1..=3.0).text("Bloom size"))
                .changed();
        });
    changed
}
