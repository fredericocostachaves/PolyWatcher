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
    SingleEvent(GammaEvent),
    OrderbookUpdate(Orderbook),
    SportsUpdate(SportsData),
    Balance(f64),
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
    auth_result_rx: Option<tokio::sync::mpsc::UnboundedReceiver<Result<(), String>>>,
    should_transition: bool,
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
            should_transition: false,
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
            should_transition: false,
        }
    }

    async fn authenticate_and_save(&self) -> Result<(), String> {
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

        // Se chegou aqui, a autenticação funcionou!
        // Obtém as credenciais finais (derivadas ou manuais)
        let final_creds = authenticated_client.credentials();

        let env_content = format!(
            "POLY_PRIVATE_KEY={}\nPOLY_FUNDER_ADDRESS={}\nPOLY_API_KEY={}\nPOLY_API_SECRET={}\nPOLY_PASSPHRASE={}\n",
            self.poly_private_key,
            self.poly_funder_address, // Salva exatamente o que o usuário forneceu (ou vazio se EOA)
            final_creds.key(),
            final_creds.secret().expose_secret(),
            final_creds.passphrase().expose_secret()
        );

        let mut file = File::create(".env")
            .map_err(|e| format!("Erro ao criar arquivo .env: {}", e))?;
        file.write_all(env_content.as_bytes())
            .map_err(|e| format!("Erro ao escrever no .env: {}", e))?;

        Ok(())
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
    balance: f64,
    wallet_address: String,
    search_global: String,
    slug_input: String,
    stake: String,
    status_log: Vec<String>,
    receiver: mpsc::Receiver<AppMessage>,
    sender: mpsc::Sender<AppMessage>,
    clob: Option<ClobClient>,
    pub logout_requested: bool,
}

impl PolyApp {
    pub fn new(clob: Option<ClobClient>) -> Self {
        let (tx, rx) = mpsc::channel(100);
        let app = Self {
            sports: Vec::new(),
            tags: HashMap::new(),
            events: Vec::new(),
            loading_tags: HashSet::new(),
            selected_event: None,
            selected_token_id: None,
            orderbook: Orderbook::default(),
            sports_updates: HashMap::new(),
            balance: 0.0,
            wallet_address: clob.as_ref().map(|c| c.creds.address.clone()).unwrap_or_default(),
            search_global: String::new(),
            slug_input: String::new(),
            stake: "10".to_string(),
            status_log: vec!["Sistema iniciado".to_string()],
            receiver: rx,
            sender: tx.clone(),
            clob,
            logout_requested: false,
        };

        app.load_initial_data();

        let tx_sports = tx.clone();
        get_runtime().spawn(async move {
            if let Err(e) = crate::sports_ws::monitor_sports_egui(tx_sports).await {
                eprintln!("Sports monitor error: {}", e);
            }
        });

        if let Some(c) = &app.clob {
            let clob = c.clone();
            let tx_bal = tx.clone();
            get_runtime().spawn(async move {
                if let Ok(b) = clob.get_balance().await {
                    let _ = tx_bal.send(AppMessage::Balance(b)).await;
                }
            });
        }

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

    fn fetch_by_slug(&mut self, slug: String) {
        let tx = self.sender.clone();
        get_runtime().spawn(async move {
            match crate::gamma::fetch_event_by_slug(slug).await {
                Ok(Some(e)) => { let _ = tx.send(AppMessage::SingleEvent(e)).await; }
                Ok(None) => { let _ = tx.send(AppMessage::Error("Slug não encontrado".to_string())).await; }
                Err(e) => { let _ = tx.send(AppMessage::Error(e.to_string())).await; }
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
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        // Verifica se existe .env com as credenciais (private key é suficiente)
        let has_credentials = std::env::var("POLY_PRIVATE_KEY").is_ok();

        let state = if has_credentials {
            // Tenta criar o ClobClient
            match ClobClient::from_env() {
                Ok(clob) => AppState::Main(PolyApp::new(Some(clob))),
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
                egui::CentralPanel::default().show(ctx, |ui| {
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
                                ui.label(RichText::new("Os campos de API são gerados automaticamente via assinatura se deixados vazios, mas isso exige que o serviço da Polymarket esteja online.").size(12.0).color(Color32::GRAY));
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

                                let button = egui::Button::new(RichText::new(button_text).size(18.0).color(Color32::WHITE))
                                    .fill(button_color)
                                    .min_size(egui::vec2(250.0, 45.0))
                                    .corner_radius(6);

                                if ui.add_enabled(!login.is_authenticating, button).clicked() {
                                    login.is_authenticating = true;
                                    login.error_message = None;

                                    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
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
                                            should_transition: false,
                                        };

                                        let result = screen.authenticate_and_save().await;
                                        let _ = tx.send(result);
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
                            Ok(_) => {
                                dotenv::dotenv().ok();
                                login.should_transition = true;
                            }
                            Err(e) => {
                                login.error_message = Some(e);
                            }
                        }
                    }
                }

                // Transiciona para a tela principal se autenticação foi bem-sucedida
                if login.should_transition {
                    match ClobClient::from_env() {
                        Ok(clob) => {
                            self.state = AppState::Main(PolyApp::new(Some(clob)));
                        }
                        Err(e) => {
                            login.should_transition = false;
                            login.error_message = Some(format!("Erro ao carregar credenciais: {}", e));
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
    fn update_impl(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
                        if !self.events.iter().any(|existing| existing.id == event.id) {
                            self.events.push(event);
                        }
                    }
                }
                AppMessage::SingleEvent(e) => {
                    if !self.events.iter().any(|existing| existing.id == e.id) {
                        self.events.push(e.clone());
                    }
                    self.selected_event = Some(e);
                    self.search_global.clear();
                }
                AppMessage::OrderbookUpdate(book) => {
                    self.orderbook = book;
                }
                AppMessage::SportsUpdate(update) => {
                    self.sports_updates.insert(update.slug.clone(), update);
                }
                AppMessage::Balance(b) => self.balance = b,
                AppMessage::Error(e) => {
                    self.status_log.push(e);
                    if self.status_log.len() > 20 { self.status_log.remove(0); }
                }
            }
        }

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("PolyWatcher").strong().size(20.0).color(Color32::from_rgb(100, 181, 246)));
                ui.separator();
                ui.label(format!("Wallet: {}", if self.wallet_address.is_empty() { "⚠️ Configuração necessária" } else { &self.wallet_address }));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("🔄").on_hover_text("Refresh Balance").clicked() {
                        if let Some(c) = &self.clob {
                            let clob = c.clone();
                            let tx_bal = self.sender.clone();
                            get_runtime().spawn(async move {
                                if let Ok(b) = clob.get_balance().await {
                                    let _ = tx_bal.send(AppMessage::Balance(b)).await;
                                }
                            });
                        }
                    }
                    ui.label(RichText::new(format!("Balance: ${:.2} USDC", self.balance)).strong().color(Color32::GREEN));
                    ui.separator();
                    if ui.button("🚪 Logout").clicked() {
                        self.logout_requested = true;
                    }
                });
            });
        });

        egui::SidePanel::left("navigator").resizable(true).default_width(250.0).show(ctx, |ui| {
            ui.heading(RichText::new("⚡ Market Navigator").color(Color32::YELLOW));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.search_global).hint_text("Search..."));
                if ui.button("Clear").clicked() { self.search_global.clear(); }
            });
            ui.separator();
            
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.slug_input).hint_text("Slug"));
                if ui.button("Load").clicked() {
                    self.fetch_by_slug(self.slug_input.clone());
                }
            });

            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                for (slug, label) in ALLOWED_LEAGUES {
                    let sport = self.sports.iter().find(|s| s.sport == *slug);
                    let tag_id = sport.and_then(|s| s.tags.first()).cloned().unwrap_or_default();

                    let header_label = format!("⚽ {}", label);
                    let collapsing = egui::CollapsingHeader::new(header_label)
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
                                if ui.button("Buscar Jogos").clicked() {
                                    self.refresh_events(tag_id.clone());
                                }
                                if self.loading_tags.contains(&tag_id) {
                                    ui.label("Carregando...");
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
                                let ev_slug = event.slug.as_deref().unwrap_or("");
                                let status = self.sports_updates.get(ev_slug).map(|u| u.status.as_str()).unwrap_or("Scheduled");
                                let color = if status == "InProgress" { Color32::from_rgb(0, 255, 0) } else { Color32::GRAY };
                                
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("●").color(color).size(10.0));
                                    if ui.selectable_label(self.selected_event.as_ref().map(|e| e.id == event.id).unwrap_or(false), title).clicked() {
                                        self.selected_event = Some(event.clone());
                                        if let Some(markets) = &event.markets {
                                            if let Some(market) = markets.first() {
                                                if let Some(tokens) = &market.clob_token_ids {
                                                    if let Some(tid) = tokens.first() {
                                                        let tid_str = tid.to_string();
                                                        self.selected_token_id = Some(tid_str.clone());
                                                        let sender = self.sender.clone();
                                                        get_runtime().spawn(async move {
                                                            let _ = crate::watcher::monitor_token_egui(&tid_str, sender).await;
                                                        });
                                                    }
                                                }
                                            }
                                        }
                                    }
                                });
                            }
                        }
                    });
                }
            });
        });

        egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Status:");
                if let Some(last) = self.status_log.last() {
                    ui.label(last);
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(event) = &self.selected_event {
                ui.heading(event.title.as_deref().unwrap_or("Untitled"));
                ui.separator();
                
                ui.horizontal(|ui| {
                    if let Some(markets) = &event.markets {
                        for market in markets {
                            if let (Some(outcomes), Some(tokens)) = (&market.outcomes, &market.clob_token_ids) {
                                for (outcome, token_id) in outcomes.iter().zip(tokens.iter()) {
                                    let tid_str = token_id.to_string();
                                    if ui.selectable_label(self.selected_token_id.as_ref() == Some(&tid_str), outcome).clicked() {
                                        self.selected_token_id = Some(tid_str.clone());
                                        let sender = self.sender.clone();
                                        get_runtime().spawn(async move {
                                            let _ = crate::watcher::monitor_token_egui(&tid_str, sender).await;
                                        });
                                    }
                                }
                            }
                        }
                    }
                });

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
                            // Por enquanto, mostra todos mas permite scroll
                            
                            // Bid
                            let bid_btn = egui::Button::new(RichText::new(&bid_size).color(Color32::BLACK))
                                .fill(if bid_size.is_empty() { Color32::TRANSPARENT } else { Color32::from_rgb(173, 216, 230) })
                                .min_size(egui::vec2(80.0, 20.0));
                            
                            if ui.add(bid_btn).clicked() {
                                self.place_order(Side::BUY, price);
                            }

                            ui.label(RichText::new(format!("{:.2} ({}¢)", odds, price_cent)).strong());

                            // Ask
                            let ask_btn = egui::Button::new(RichText::new(&ask_size).color(Color32::BLACK))
                                .fill(if ask_size.is_empty() { Color32::TRANSPARENT } else { Color32::from_rgb(255, 182, 193) })
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
                    ui.label("Selecione um evento para começar");
                });
            }
        });
        
        ctx.request_repaint();
    }
}
