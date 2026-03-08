mod clob;
mod gamma;
mod watcher;
mod sports_ws;
mod ui;

use dotenv::dotenv;
use std::env;
use crate::clob::{ClobClient, Credentials};
use crate::ui::PolyApp;

#[tokio::main]
async fn main() -> Result<(), eframe::Error> {
    dotenv().ok();

    // Requerido para rustls v0.23+ quando múltiplas features de crypto estão presentes
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let clob = if let (Ok(addr), Ok(api_key), Ok(api_secret), Ok(pass), Ok(priv_key)) = (
        env::var("POLY_ADDRESS"),
        env::var("POLY_API_KEY"),
        env::var("POLY_API_SECRET"),
        env::var("POLY_PASSPHRASE"),
        env::var("POLY_PRIVATE_KEY"),
    ) {
        let funder = env::var("POLY_FUNDER_ADDRESS").ok();
        Some(ClobClient::new(Credentials {
            address: addr,
            api_key,
            api_secret,
            passphrase: pass,
            private_key: priv_key,
            funder_address: funder,
        }))
    } else {
        None
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PolyWatcher - Pro Trading Terminal")
            .with_inner_size([1200.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native(
        "PolyWatcher",
        options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(PolyApp::new(cc, clob)))
        }),
    )
}
