mod app;
mod document;
mod markdown;

use app::MarkGuin;
use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([820.0, 560.0])
            .with_title("MarkGuin"),
        ..Default::default()
    };

    eframe::run_native(
        "MarkGuin",
        options,
        Box::new(|cc| Ok(Box::new(MarkGuin::new(cc)))),
    )
}
