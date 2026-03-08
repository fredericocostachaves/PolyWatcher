mod clob;
mod gamma;
mod watcher;
mod sports_ws;
mod ui;

use dotenv::dotenv;
use crate::ui::App;
use std::sync::OnceLock;

pub static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn main() -> eframe::Result<()> {
    dotenv().ok();

    // Requerido para rustls v0.23+ quando múltiplas features de crypto estão presentes
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    // Inicializa o runtime global do Tokio
    RUNTIME.get_or_init(|| {
        tokio::runtime::Runtime::new().unwrap()
    });

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "PolyWatcher - Login",
        native_options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
