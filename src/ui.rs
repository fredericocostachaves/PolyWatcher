use eframe::egui;
use egui::{Color32, RichText, Layout, Align};
use std::collections::{HashMap, HashSet};
use crate::gamma::{GammaEvent, GammaSport, GammaTag};
use crate::clob::{ClobClient, Side};
use crate::watcher::Orderbook;
use crate::sports_ws::SportsData;
use tokio::sync::mpsc;

fn get_runtime() -> &'static tokio::runtime::Runtime {
    crate::RUNTIME.get().expect("Runtime not initialized")
}

pub enum AppMessage {
    Sports(Vec<GammaSport>),
    Tags(Vec<GammaTag>),
    Events(Vec<GammaEvent>, Option<String>),
    OrderbookUpdate(Orderbook, String),
    SportsUpdate(SportsData),
    TotalValue(f64),
    Error(String),
}

pub struct LoginScreen {
    poly_private_key: String,
    poly_funder_address: String,
    poly_api_key: String,
    poly_api_secret: String,
    poly_passphrase: String,
    error_message: Option<String>,
    is_authenticating: bool,
    auth_result_rx: Option<mpsc::UnboundedReceiver<Result<(ClobClient, f64), String>>>,
}

impl Default for LoginScreen {
    fn default() -> Self {
        Self {
            poly_private_key: String::new(),
            poly_funder_address: String::new(),
            poly_api_key: String::new(),
            poly_api_secret: String::new(),
            poly_passphrase: String::new(),
            error_message: None,
            is_authenticating: false,
            auth_result_rx: None,
        }
    }
}

impl LoginScreen {
    pub fn new_from_env() -> Self {
        use std::env;
        Self {
            poly_private_key: env::var("POLY_PRIVATE_KEY").unwrap_or_default(),
            poly_funder_address: env::var("POLY_FUNDER_ADDRESS").unwrap_or_default(),
            poly_api_key: env::var("POLY_API_KEY").unwrap_or_default(),
            poly_api_secret: env::var("POLY_API_SECRET").unwrap_or_default(),
            poly_passphrase: env::var("POLY_PASSPHRASE").unwrap_or_default(),
            error_message: None,
            is_authenticating: false,
            auth_result_rx: None,
        }
    }

    async fn authenticate_and_save(&self) -> Result<ClobClient, String> {
        use std::fs::File;
        use std::io::Write;
        use alloy::signers::local::PrivateKeySigner;
        use alloy::signers::Signer;
        use std::str::FromStr;
        use polymarket_client_sdk::clob::{Client as SdkClient, Config as SdkConfig};
        use polymarket_client_sdk::clob::types::SignatureType;
        use polymarket_client_sdk::POLYGON;
        use polymarket_client_sdk::auth::Credentials as SdkCredentials;
        use polymarket_client_sdk::auth::ExposeSecret;
        use uuid::Uuid;

        // Cria o signer a partir da private key
        let signer = PrivateKeySigner::from_str(&self.poly_private_key)
            .map_err(|e| format!("Private key inválida: {}", e))?
            .with_chain_id(Some(POLYGON));


        // Cria o cliente para autenticação
        let client = SdkClient::new("https://clob.polymarket.com", SdkConfig::default())
            .map_err(|e| format!("Erro ao criar cliente: {}", e))?;

        // Cria o builder de autenticação
        let mut auth_builder = client.authentication_builder(&signer);

        // Se o usuário forneceu as credenciais da API manualmente, usa-as
        let has_api_creds = !self.poly_api_key.is_empty() && !self.poly_api_secret.is_empty() && !self.poly_passphrase.is_empty();

        if !has_api_creds && (!self.poly_api_key.is_empty() || !self.poly_api_secret.is_empty() || !self.poly_passphrase.is_empty()) {
            return Err("Para usar chaves manuais, você deve preencher os 3 campos: API Key, Secret e Passphrase.".to_string());
        }

        let mut _manual_creds = None;

        if has_api_creds {
            let api_key_uuid = Uuid::parse_str(&self.poly_api_key)
                .map_err(|e| format!("API Key inválida (deve ser UUID): {}", e))?;
            
            let sdk_creds = SdkCredentials::new(
                api_key_uuid,
                self.poly_api_secret.clone(),
                self.poly_passphrase.clone(),
            );
            auth_builder = auth_builder.credentials(sdk_creds.clone());
            _manual_creds = Some(sdk_creds);
        }

        // Configura signature type e funder
        if !self.poly_funder_address.is_empty() {
            if let Ok(funder_addr) = alloy::primitives::Address::from_str(&self.poly_funder_address) {
                auth_builder = auth_builder
                    .signature_type(SignatureType::GnosisSafe)
                    .funder(funder_addr);
            } else {
                return Err("Endereço do funder inválido".to_string());
            }
        } else {
            // Se não tem funder, usa EOA (signature type 0)
            auth_builder = auth_builder.signature_type(SignatureType::Eoa);
        }

        // Testa a autenticação (cria ou deriva as credenciais automaticamente)
        let authenticated_client = auth_builder.authenticate()
            .await
            .map_err(|e| {
                if e.to_string().contains("503") {
                    format!("Erro 503: O serviço de derivação está instável. Insira API Key, Secret e Passphrase manualmente para conectar.\n{}", e)
                } else {
                    format!("Erro ao autenticar: {}", e)
                }
            })?;

        let final_creds = authenticated_client.credentials();

        let clob = ClobClient::new(crate::clob::Credentials {
            address: if !self.poly_funder_address.is_empty() {
                self.poly_funder_address.to_lowercase()
            } else {
                format!("{:#x}", signer.address()).to_lowercase()
            },
            api_key: final_creds.key().to_string(),
            api_secret: final_creds.secret().expose_secret().to_string(),
            passphrase: final_creds.passphrase().expose_secret().to_string(),
            private_key: self.poly_private_key.clone(),
            funder_address: if self.poly_funder_address.is_empty() { None } else { Some(self.poly_funder_address.to_lowercase()) },
        });

        let env_content = format!(
            "POLY_PRIVATE_KEY={}\nPOLY_FUNDER_ADDRESS={}\nPOLY_API_KEY={}\nPOLY_API_SECRET={}\nPOLY_PASSPHRASE={}\n",
            clob.creds.private_key,
            clob.creds.funder_address.as_deref().unwrap_or(""),
            clob.creds.api_key,
            clob.creds.api_secret,
            clob.creds.passphrase
        );

        let mut file = File::create(".env")
            .map_err(|e| format!("Erro ao criar arquivo .env: {}", e))?;
        file.write_all(env_content.as_bytes())
            .map_err(|e| format!("Erro ao escrever no .env: {}", e))?;

        Ok(clob)
    }
}

pub enum AppState {
    Login(LoginScreen),
    Main(PolyApp),
}

pub struct App {
    state: AppState,
}

pub struct PolyApp {
    sports: Vec<GammaSport>,
    tags: HashMap<String, String>,
    events: Vec<GammaEvent>,
    loading_tags: HashSet<String>,
    selected_event: Option<GammaEvent>,
    selected_token_id: Option<String>,
    orderbook: Orderbook,
    sports_updates: HashMap<String, SportsData>,
    total_value: f64,
    wallet_address: String,
    search_global: String,
    stake: String,
    status_log: Vec<String>,
    receiver: mpsc::Receiver<AppMessage>,
    sender: mpsc::Sender<AppMessage>,
    clob: Option<ClobClient>,
    pub logout_requested: bool,
}

impl PolyApp {
    pub fn new(clob: Option<ClobClient>, initial_total_value: Option<f64>) -> Self {
        let (tx, rx) = mpsc::channel(100);
        let total_value = initial_total_value.unwrap_or(0.0);
        let mut app = Self {
            sports: Vec::new(),
            tags: HashMap::new(),
            events: Vec::new(),
            loading_tags: HashSet::new(),
            selected_event: None,
            selected_token_id: None,
            orderbook: Orderbook::default(),
            sports_updates: HashMap::new(),
            total_value,
            wallet_address: clob.as_ref().map(|c| c.creds.address.clone()).unwrap_or_default(),
            search_global: String::new(),
            stake: "10".to_string(),
            status_log: vec!["Sistema iniciado".to_string()],
            receiver: rx,
            sender: tx.clone(),
            clob,
            logout_requested: false,
        };

        app.load_initial_data();
        if initial_total_value.is_none() {
            app.refresh_total_value();
        }

        let tx_sports = tx.clone();
        get_runtime().spawn(async move {
            if let Err(e) = crate::sports_ws::monitor_sports_egui(tx_sports).await {
                eprintln!("Sports monitor error: {}", e);
            }
        });

        app
    }

    fn load_initial_data(&self) {
        let tx = self.sender.clone();
        get_runtime().spawn(async move {
            match crate::gamma::fetch_sports().await {
                Ok(s) => { let _ = tx.send(AppMessage::Sports(s)).await; }
                Err(e) => { let _ = tx.send(AppMessage::Error(e.to_string())).await; }
            }
            match crate::gamma::fetch_tags().await {
                Ok(t) => { let _ = tx.send(AppMessage::Tags(t)).await; }
                Err(e) => { let _ = tx.send(AppMessage::Error(e.to_string())).await; }
            }
            match crate::gamma::fetch_events(Some("100350".to_string())).await {
                Ok(e) => { let _ = tx.send(AppMessage::Events(e, Some("100350".to_string()))).await; }
                Err(e) => { let _ = tx.send(AppMessage::Error(e.to_string())).await; }
            }
        });
    }

    fn refresh_total_value(&mut self) {
        if let Some(clob) = &self.clob {
            // Valida se POLY_FUNDER_ADDRESS está preenchido caso seja necessário
            if clob.creds.funder_address.is_none() || clob.creds.funder_address.as_ref().map(|s| s.is_empty()).unwrap_or(true) {
                self.status_log.push("⚠️ POLY_FUNDER_ADDRESS não está preenchido. Retornando ao login.".to_string());
                self.logout_requested = true;
                return;
            }

            let clob = clob.clone();
            let tx = self.sender.clone();
            get_runtime().spawn(async move {
                match clob.get_total_value().await {
                    Ok(v) => { let _ = tx.send(AppMessage::TotalValue(v)).await; }
                    Err(e) => { let _ = tx.send(AppMessage::Error(format!("Erro ao buscar valor total: {}", e))).await; }
                }
            });
        }
    }

    fn refresh_events(&mut self, tag_id: String) {
        if self.loading_tags.contains(&tag_id) { return; }
        self.loading_tags.insert(tag_id.clone());
        let tx = self.sender.clone();
        let tid = tag_id.clone();
        get_runtime().spawn(async move {
            match crate::gamma::fetch_events(Some(tid.clone())).await {
                Ok(e) => { let _ = tx.send(AppMessage::Events(e, Some(tid))).await; }
                Err(e) => {
                    let _ = tx.send(AppMessage::Error(e.to_string())).await;
                    let _ = tx.send(AppMessage::Events(vec![], Some(tid))).await;
                }
            }
        });
    }

    fn place_order(&mut self, side: Side, price: f64) {
        let Some(clob) = self.clob.as_ref() else {
            self.status_log.push("Erro: Clob não configurado".to_string());
            return;
        };
        let Some(token_id) = self.selected_token_id.clone() else { return; };
        let size = self.stake.parse::<f64>().unwrap_or(10.0);
        let tx = self.sender.clone();
        let clob = clob.clone();

        get_runtime().spawn(async move {
            match clob.post_order(token_id, side, price, size).await {
                Ok(resp) => { let _ = tx.send(AppMessage::Error(format!("Ordem enviada: {}", resp))).await; }
                Err(e) => { let _ = tx.send(AppMessage::Error(format!("Erro na ordem: {}", e))).await; }
            }
        });
    }
}

const ALLOWED_LEAGUES: &[(&str, &str)] = &[
    ("ucl", "Champions League"),
    ("bun", "Bundesliga"),
    ("lal", "La Liga"),
    ("sea", "Serie A"),
    ("mls", "MLS"),
    ("fl1", "Ligue 1"),
    ("tur", "Süper Lig"),
    ("mex", "Liga MX"),
    ("por", "Primeira Liga"),
    ("ere", "Eredivisie"),
    ("spl", "Saudi Pro League"),
    ("epl", "Premier League"),
    ("bra", "Brazil Série A"),
    ("uel", "Europa League"),
    ("lib", "Libertadores"),
    ("arg", "Argentina Primera"),
];

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Configura um tema escuro customizado com fundo bem escuro para garantir contraste
        let mut visuals = egui::Visuals::dark();
        visuals.override_text_color = Some(Color32::from_rgb(230, 230, 230)); // Força cor de texto clara globalmente
        visuals.panel_fill = Color32::from_rgb(15, 17, 20);  // Fundo muito escuro para os painéis
        visuals.window_fill = Color32::from_rgb(25, 27, 31); // Fundo consistente com o frame de login
        
        // Melhora o contraste dos botões padrão (widgets)
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(45, 47, 51);
        visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, Color32::from_rgb(230, 230, 230)); // Cor da seta e bordas
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(65, 67, 71);
        visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.5, Color32::WHITE);
        visuals.widgets.active.bg_fill = Color32::from_rgb(85, 87, 91);
        visuals.widgets.active.fg_stroke = egui::Stroke::new(1.5, Color32::WHITE);
        
        // Contraste para widgets não interativos (como a seta quando não estamos interagindo)
        visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, Color32::from_rgb(210, 210, 210));
        
        cc.egui_ctx.set_visuals(visuals);

        // Verifica se existe .env com as credenciais (private key é suficiente)
        let has_credentials = std::env::var("POLY_PRIVATE_KEY").is_ok();

        let state = if has_credentials {
            // Tenta criar o ClobClient
            match ClobClient::from_env() {
                Ok(clob) => AppState::Main(PolyApp::new(Some(clob), None)),
                Err(_) => AppState::Login(LoginScreen::new_from_env()),
            }
        } else {
            AppState::Login(LoginScreen::new_from_env())
        };

        Self { state }
    }

}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        match &mut self.state {
            AppState::Login(login) => {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(Color32::from_rgb(15, 17, 20)))
                    .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() * 0.07);

                        let frame = egui::Frame::window(ui.style())
                            .fill(Color32::from_rgb(25, 27, 31))
                            .corner_radius(12)
                            .shadow(egui::Shadow {
                                color: Color32::from_black_alpha(150),
                                offset: [0, 15],
                                blur: 30,
                                spread: 0,
                            })
                            .inner_margin(egui::Margin::same(30));

                        frame.show(ui, |ui| {
                            ui.set_max_width(500.0);

                            ui.vertical_centered(|ui| {
                                ui.add_space(10.0);
                                ui.heading(RichText::new("🔐 PolyWatcher").size(36.0).strong().color(Color32::from_rgb(100, 181, 246)));
                                ui.add_space(10.0);
                                ui.label(RichText::new("Configuração de Credenciais").size(16.0).color(Color32::LIGHT_GRAY));
                                ui.add_space(30.0);

                                egui::Grid::new("login_grid")
                                    .num_columns(2)
                                    .spacing([15.0, 20.0])
                                    .show(ui, |ui| {
                                        ui.label(RichText::new("Private Key:").size(15.0).color(Color32::WHITE));
                                        ui.add(egui::TextEdit::singleline(&mut login.poly_private_key)
                                            .desired_width(300.0)
                                            .password(true)
                                            .hint_text("0x..."));
                                        ui.end_row();

                                        ui.label(RichText::new("Funder Address:").size(15.0).color(Color32::WHITE));
                                        ui.add(egui::TextEdit::singleline(&mut login.poly_funder_address)
                                            .desired_width(300.0)
                                            .hint_text("0x... (opcional)"));
                                        ui.end_row();

                                        ui.label(RichText::new("API Key:").size(15.0).color(Color32::WHITE));
                                        ui.add(egui::TextEdit::singleline(&mut login.poly_api_key)
                                            .desired_width(300.0)
                                            .hint_text("UUID (Necessário se a derivação falhar)"));
                                        ui.end_row();

                                        ui.label(RichText::new("API Secret:").size(15.0).color(Color32::WHITE));
                                        ui.add(egui::TextEdit::singleline(&mut login.poly_api_secret)
                                            .desired_width(300.0)
                                            .password(true)
                                            .hint_text("Secret (opcional)"));
                                        ui.end_row();

                                        ui.label(RichText::new("Passphrase:").size(15.0).color(Color32::WHITE));
                                        ui.add(egui::TextEdit::singleline(&mut login.poly_passphrase)
                                            .desired_width(300.0)
                                            .password(true)
                                            .hint_text("Passphrase (opcional)"));
                                        ui.end_row();
                                    });

                                ui.add_space(30.0);
                                ui.label(RichText::new("Os campos de API são gerados automaticamente via assinatura se deixados vazios, mas isso exige que o serviço da Polymarket esteja online.").size(12.0).color(Color32::from_rgb(180, 180, 180)));
                                ui.add_space(30.0);

                                let button_text = if login.is_authenticating {
                                    "⏳ Autenticando..."
                                } else {
                                    "🔐 Autenticar e Conectar"
                                };

                                let button_color = if login.is_authenticating {
                                    Color32::from_rgb(60, 60, 60)
                                } else {
                                    Color32::from_rgb(33, 150, 243)
                                };

                                let button = egui::Button::new(RichText::new(button_text).size(18.0).color(Color32::WHITE).strong())
                                    .fill(button_color)
                                    .min_size(egui::vec2(250.0, 45.0))
                                    .corner_radius(6);

                                if ui.add_enabled(!login.is_authenticating, button).clicked() {
                                    login.is_authenticating = true;
                                    login.error_message = None;

                                    let (tx, rx) = mpsc::unbounded_channel();
                                    login.auth_result_rx = Some(rx);

                                    let private_key = login.poly_private_key.clone();
                                    let funder_address = login.poly_funder_address.clone();
                                    let api_key = login.poly_api_key.clone();
                                    let api_secret = login.poly_api_secret.clone();
                                    let passphrase = login.poly_passphrase.clone();

                                    get_runtime().spawn(async move {
                                        let screen = LoginScreen {
                                            poly_private_key: private_key,
                                            poly_funder_address: funder_address,
                                            poly_api_key: api_key,
                                            poly_api_secret: api_secret,
                                            poly_passphrase: passphrase,
                                            error_message: None,
                                            is_authenticating: false,
                                            auth_result_rx: None,
                                        };

                                        match screen.authenticate_and_save().await {
                                            Ok(clob) => {
                                                match clob.get_total_value().await {
                                                    Ok(total_value) => {
                                                        let _ = tx.send(Ok((clob, total_value)));
                                                    }
                                                    Err(e) => {
                                                        let _ = tx.send(Err(format!("Autenticado, mas erro ao buscar valor total: {}", e)));
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                let _ = tx.send(Err(e));
                                            }
                                        }
                                    });
                                }

                                if let Some(error) = &login.error_message {
                                    ui.add_space(15.0);
                                    let color = if error.starts_with("✅") {
                                        Color32::from_rgb(76, 175, 80)
                                    } else {
                                        Color32::from_rgb(244, 67, 54)
                                    };
                                    ui.label(RichText::new(error).color(color));
                                }

                                if login.is_authenticating {
                                    ui.add_space(15.0);
                                    ui.label(RichText::new("Isso pode levar alguns segundos...").color(Color32::from_rgb(255, 213, 79)));
                                }
                                ui.add_space(10.0);
                            });
                        });
                    });
                });

                // Verifica resultado da autenticação
                if let Some(rx) = &mut login.auth_result_rx {
                    if let Ok(result) = rx.try_recv() {
                        login.is_authenticating = false;
                        login.auth_result_rx = None;

                        match result {
                            Ok((clob, total_value)) => {
                                self.state = AppState::Main(PolyApp::new(Some(clob), Some(total_value)));
                            }
                            Err(e) => {
                                login.error_message = Some(e);
                            }
                        }
                    }
                }
            }
            AppState::Main(poly_app) => {
                poly_app.update_impl(ctx, frame);
                if poly_app.logout_requested {
                    let mut login = LoginScreen::default();
                    if let Some(clob) = &poly_app.clob {
                        login.poly_private_key = clob.creds.private_key.clone();
                        login.poly_funder_address = clob.creds.funder_address.clone().unwrap_or_default();
                        login.poly_api_key = clob.creds.api_key.clone();
                        login.poly_api_secret = clob.creds.api_secret.clone();
                        login.poly_passphrase = clob.creds.passphrase.clone();
                    }
                    self.state = AppState::Login(login);
                }
            }
        }
    }
}

impl PolyApp {
    fn select_token(&mut self, token_id: String) {
        self.selected_token_id = Some(token_id.clone());
        self.orderbook = Orderbook::default(); // Limpa o book ao trocar de token
        let sender = self.sender.clone();
        get_runtime().spawn(async move {
            let _ = crate::watcher::monitor_token_egui(&token_id, sender).await;
        });
    }

    fn select_event(&mut self, event: GammaEvent) {
        self.selected_event = Some(event.clone());
        if let Some(market) = event.markets.as_ref().and_then(|m| m.first()) {
            if let Some(tid) = market.clob_token_ids.as_ref().and_then(|t| t.first()) {
                self.select_token(tid.to_string());
            }
        }
    }

    fn draw_event_item(&self, ui: &mut egui::Ui, event: &GammaEvent) -> bool {
        let title = event.title.as_deref().unwrap_or("Untitled");
        let ev_slug = event.slug.as_deref().unwrap_or("");
        let status = self.sports_updates.get(ev_slug).map(|u| u.status.as_str()).unwrap_or("Scheduled");
        let color = if status == "InProgress" { Color32::from_rgb(0, 255, 0) } else { Color32::LIGHT_GRAY };
        let is_selected = self.selected_event.as_ref().map(|e| e.id == event.id).unwrap_or(false);

        let mut clicked = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new("●").color(color).size(10.0));
            if ui.selectable_label(is_selected, RichText::new(title).color(Color32::WHITE).strong()).clicked() {
                clicked = true;
            }
        });
        clicked
    }

    fn update_impl(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut clicked_event = None;
        let mut clicked_token = None;

        while let Ok(msg) = self.receiver.try_recv() {
            match msg {
                AppMessage::Sports(s) => self.sports = s,
                AppMessage::Tags(t) => {
                    for tag in t {
                        if let Some(label) = tag.label {
                            self.tags.insert(tag.id.clone(), label);
                        }
                    }
                }
                AppMessage::Events(e, tid) => {
                    if let Some(id) = tid { self.loading_tags.remove(&id); }
                    for event in e {
                        if let Some(existing) = self.events.iter_mut().find(|ex| ex.id == event.id) {
                            *existing = event;
                        } else {
                            self.events.push(event);
                        }
                    }
                }
                AppMessage::OrderbookUpdate(book, tid) => {
                    if self.selected_token_id.as_ref() == Some(&tid) {
                        self.orderbook = book;
                    }
                }
                AppMessage::SportsUpdate(update) => {
                    self.sports_updates.insert(update.slug.clone(), update);
                }
                AppMessage::TotalValue(v) => self.total_value = v,
                AppMessage::Error(e) => {
                    self.status_log.push(e);
                    if self.status_log.len() > 20 { self.status_log.remove(0); }
                }
            }
        }

        egui::TopBottomPanel::top("header")
            .frame(egui::Frame::NONE.fill(Color32::from_rgb(25, 27, 31)).inner_margin(8.0))
            .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("PolyWatcher").strong().size(20.0).color(Color32::from_rgb(100, 181, 246)));
                ui.separator();
                ui.label(RichText::new(format!("Wallet: {}", if self.wallet_address.is_empty() { "⚠️ Configuração necessária" } else { &self.wallet_address })).color(Color32::LIGHT_GRAY));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button(RichText::new("🔄").color(Color32::BLACK)).on_hover_text("Atualizar valor total das posições").clicked() {
                        self.refresh_total_value();
                    }
                    ui.label(RichText::new(format!("Valor Total Posicionado: ${:.2}", self.total_value)).strong().color(Color32::GREEN));
                    ui.separator();
                    if ui.button(RichText::new("🚪 Logout").color(Color32::BLACK).strong()).clicked() {
                        self.logout_requested = true;
                    }
                });
            });
        });

        egui::SidePanel::left("navigator")
            .frame(egui::Frame::NONE.fill(Color32::from_rgb(20, 22, 26)).inner_margin(12.0))
            .resizable(true)
            .default_width(250.0)
            .show(ctx, |ui| {
            ui.heading(RichText::new("⚡ Navegar pelo Mercado").color(Color32::YELLOW));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.search_global).hint_text(RichText::new("Search...").color(Color32::GRAY)));
                if ui.button(RichText::new("Clear").color(Color32::BLACK).strong()).clicked() { self.search_global.clear(); }
            });
            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                if !self.search_global.is_empty() {
                    ui.label(RichText::new("🔍 Search Results").color(Color32::from_rgb(100, 181, 246)).strong());
                    let search_query = self.search_global.to_lowercase();
                    let mut found_any = false;
                    
                    for event in &self.events {
                        let title = event.title.as_deref().unwrap_or("Untitled");
                        if title.to_lowercase().contains(&search_query) {
                            found_any = true;
                            ui.push_id(&event.id, |ui| {
                                if self.draw_event_item(ui, event) {
                                    clicked_event = Some(event.clone());
                                }
                            });
                        }
                    }
                    
                    if !found_any {
                        ui.label(RichText::new("No markets found").italics().color(Color32::GRAY));
                    }
                    ui.separator();
                }

                for (slug, label) in ALLOWED_LEAGUES {
                    let sport = self.sports.iter().find(|s| s.sport == *slug);
                    let tag_id = sport.and_then(|s| s.tags.last()).cloned().unwrap_or_default(); // Usa o último tag (mais específico)

                    let header_label = format!("⚽ {}", label);
                    let collapsing = egui::CollapsingHeader::new(RichText::new(header_label).color(Color32::WHITE).strong())
                        .id_salt(format!("league_{}", slug));
                    
                    collapsing.show(ui, |ui| {
                        if !tag_id.is_empty() {
                            let has_events = self.events.iter().any(|e| {
                                if let Some(tags) = &e.tags {
                                    tags.iter().any(|t| t.id == tag_id)
                                } else {
                                    false
                                }
                            });
                            if !has_events {
                                if ui.button(RichText::new("Buscar Jogos").color(Color32::WHITE).strong()).clicked() {
                                    self.refresh_events(tag_id.clone());
                                }
                                if self.loading_tags.contains(&tag_id) {
                                    ui.label(RichText::new("Carregando...").color(Color32::LIGHT_GRAY));
                                }
                            }
                        }

                        for event in &self.events {
                            let match_tag = if let Some(tags) = &event.tags {
                                tags.iter().any(|t| t.id == tag_id)
                            } else {
                                false
                            };
                            let title = event.title.as_deref().unwrap_or("Untitled");
                            let match_search = self.search_global.is_empty() || 
                                title.to_lowercase().contains(&self.search_global.to_lowercase());
                            
                            if match_tag && match_search {
                                ui.push_id(&event.id, |ui| {
                                    if self.draw_event_item(ui, event) {
                                        clicked_event = Some(event.clone());
                                    }
                                });
                            }
                        }
                    });
                }
            });
        });

        egui::TopBottomPanel::bottom("footer")
            .frame(egui::Frame::NONE.fill(Color32::from_rgb(25, 27, 31)).inner_margin(4.0))
            .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Status:").color(Color32::LIGHT_GRAY));
                if let Some(last) = self.status_log.last() {
                    ui.label(RichText::new(last).color(Color32::WHITE));
                }
            });
        });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(Color32::from_rgb(15, 17, 20)).inner_margin(20.0))
            .show(ctx, |ui| {
            if let Some(event) = &self.selected_event {
                ui.heading(RichText::new(event.title.as_deref().unwrap_or("Untitled")).color(Color32::WHITE));
                ui.separator();
                
                if let Some(markets) = &event.markets {
                    for market in markets {
                        ui.label(RichText::new(market.question.as_deref().unwrap_or("Mercado")).color(Color32::YELLOW).small());
                        ui.horizontal(|ui| {
                            if let (Some(outcomes), Some(tokens)) = (&market.outcomes, &market.clob_token_ids) {
                                for (outcome, token_id) in outcomes.iter().zip(tokens.iter()) {
                                    let tid_str = token_id.to_string();
                                    if ui.selectable_label(self.selected_token_id.as_ref() == Some(&tid_str), RichText::new(outcome).color(Color32::WHITE).strong()).clicked() {
                                        clicked_token = Some(tid_str);
                                    }
                                }
                            }
                        });
                        ui.add_space(4.0);
                    }
                }

                ui.separator();

                let book = self.orderbook.clone();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::Grid::new("ladder").striped(true).show(ui, |ui| {
                        ui.label(RichText::new("Back (Bid)").strong().color(Color32::from_rgb(33, 150, 243)));
                        ui.label(RichText::new("ODDS").strong().color(Color32::WHITE));
                        ui.label(RichText::new("Lay (Ask)").strong().color(Color32::from_rgb(244, 67, 54)));
                        ui.end_row();

                        for price_cent in (1..100).rev() {
                            let price = price_cent as f64 / 100.0;
                            let odds = if price > 0.0 { 1.0 / price } else { 0.0 };
                            
                            let bid_size = book.bids.get(&price_cent).cloned().unwrap_or_default();
                            let ask_size = book.asks.get(&price_cent).cloned().unwrap_or_default();

                            // Pula níveis sem liquidez se estiverem longe do spread? 
                            // Por enquanto, mostra todos, mas permite scroll
                            
                            // Bid
                            let bid_btn = egui::Button::new(RichText::new(&bid_size).color(Color32::WHITE).strong())
                                .fill(if bid_size.is_empty() { Color32::TRANSPARENT } else { Color32::from_rgb(25, 118, 210) })
                                .min_size(egui::vec2(80.0, 20.0));
                            
                            if ui.add(bid_btn).clicked() {
                                self.place_order(Side::BUY, price);
                            }

                            ui.label(RichText::new(format!("{:.2} ({}¢)", odds, price_cent)).strong().color(Color32::WHITE));

                            // Ask
                            let ask_btn = egui::Button::new(RichText::new(&ask_size).color(Color32::WHITE).strong())
                                .fill(if ask_size.is_empty() { Color32::TRANSPARENT } else { Color32::from_rgb(198, 40, 40) })
                                .min_size(egui::vec2(80.0, 20.0));
                            
                            if ui.add(ask_btn).clicked() {
                                self.place_order(Side::SELL, price);
                            }
                            ui.end_row();
                        }
                    });
                });
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("Selecione um evento para começar").color(Color32::LIGHT_GRAY).size(18.0));
                });
            }
        });
        
        if let Some(event) = clicked_event {
            self.select_event(event);
        }
        if let Some(tid) = clicked_token {
            self.select_token(tid);
        }

        ctx.request_repaint();
    }
}
