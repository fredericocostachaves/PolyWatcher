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
            error_message: None,
            is_authenticating: false,
            auth_result_rx: None,
            should_transition: false,
        }
    }
}

impl LoginScreen {
    async fn authenticate_and_save(&self) -> Result<(), String> {
        use std::fs::File;
        use std::io::Write;
        use alloy::signers::local::PrivateKeySigner;
        use alloy::signers::Signer;
        use std::str::FromStr;
        use polymarket_client_sdk::clob::{Client as SdkClient, Config as SdkConfig};
        use polymarket_client_sdk::clob::types::SignatureType;
        use polymarket_client_sdk::POLYGON;

        // Cria o signer a partir da private key
        let signer = PrivateKeySigner::from_str(&self.poly_private_key)
            .map_err(|e| format!("Private key inválida: {}", e))?
            .with_chain_id(Some(POLYGON));

        // Obtém o endereço da carteira
        let address = signer.address();

        // Cria o cliente para autenticação
        let client = SdkClient::new("https://clob.polymarket.com", SdkConfig::default())
            .map_err(|e| format!("Erro ao criar cliente: {}", e))?;

        // Cria o builder de autenticação
        let mut auth_builder = client.authentication_builder(&signer);

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
        let _authenticated_client = auth_builder.authenticate()
            .await
            .map_err(|e| format!("Erro ao autenticar: {}", e))?;

        // Se chegou aqui, a autenticação funcionou!
        // Salva apenas o necessário no .env - o SDK gerencia as credenciais da API
        let funder_str = if self.poly_funder_address.is_empty() {
            format!("{:?}", address) // Usa o próprio endereço como funder
        } else {
            self.poly_funder_address.clone()
        };

        let env_content = format!(
            "POLY_PRIVATE_KEY={}\nPOLY_FUNDER_ADDRESS={}\n",
            self.poly_private_key,
            funder_str
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
}

impl PolyApp {
    pub fn new(_cc: &eframe::CreationContext<'_>, clob: Option<ClobClient>) -> Self {
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
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Verifica se existe .env com as credenciais (private key é suficiente)
        let has_credentials = std::env::var("POLY_PRIVATE_KEY").is_ok();

        let state = if has_credentials {
            // Tenta criar o ClobClient
            match ClobClient::from_env() {
                Ok(clob) => AppState::Main(PolyApp::new(cc, Some(clob))),
                Err(_) => AppState::Login(LoginScreen::default()),
            }
        } else {
            AppState::Login(LoginScreen::default())
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
                        ui.add_space(50.0);
                        ui.heading(RichText::new("🔐 PolyWatcher Login").size(32.0).color(Color32::from_rgb(0, 255, 255)));
                        ui.add_space(20.0);
                        ui.label("Configure suas credenciais da Polymarket");
                        ui.add_space(30.0);
                    });

                    ui.add_space(20.0);

                    egui::Grid::new("login_grid")
                        .num_columns(2)
                        .spacing([40.0, 15.0])
                        .show(ui, |ui| {
                            ui.label("Private Key:");
                            ui.add(egui::TextEdit::singleline(&mut login.poly_private_key)
                                .desired_width(500.0)
                                .password(true)
                                .hint_text("0x..."));
                            ui.end_row();

                            ui.label("Funder Address (opcional):");
                            ui.add(egui::TextEdit::singleline(&mut login.poly_funder_address)
                                .desired_width(500.0)
                                .hint_text("0x..."));
                            ui.end_row();
                        });

                    ui.add_space(20.0);

                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("ℹ️ As credenciais da API serão geradas automaticamente").size(12.0).color(Color32::GRAY));
                    });

                    ui.add_space(20.0);

                    ui.vertical_centered(|ui| {
                        let button_text = if login.is_authenticating {
                            "⏳ Autenticando..."
                        } else {
                            "🔐 Autenticar e Conectar"
                        };

                        let button = egui::Button::new(RichText::new(button_text).size(18.0))
                            .min_size(egui::vec2(250.0, 45.0));

                        if ui.add_enabled(!login.is_authenticating, button).clicked() {
                            login.is_authenticating = true;
                            login.error_message = None;

                            // Cria canal para receber resultado
                            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                            login.auth_result_rx = Some(rx);

                            // Clona os dados necessários para a task assíncrona
                            let private_key = login.poly_private_key.clone();
                            let funder_address = login.poly_funder_address.clone();

                            // Executa a autenticação em background
                            get_runtime().spawn(async move {
                                let screen = LoginScreen {
                                    poly_private_key: private_key,
                                    poly_funder_address: funder_address,
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
                            ui.add_space(10.0);
                            let color = if error.starts_with("✅") {
                                Color32::GREEN
                            } else {
                                Color32::RED
                            };
                            ui.label(RichText::new(error).color(color));
                        }

                        if login.is_authenticating {
                            ui.add_space(10.0);
                            ui.label(RichText::new("Aguarde... Isso pode levar alguns segundos.").color(Color32::YELLOW));
                        }
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
                        Ok(_clob) => {
                            // Não podemos criar PolyApp aqui sem CreationContext
                            // Solução: salvar e pedir para reiniciar
                            login.should_transition = false;
                            login.error_message = Some("✅ Autenticação bem-sucedida! Reinicie o aplicativo.".to_string());

                            // Aguarda 2 segundos e fecha
                            let ctx_clone = ctx.clone();
                            get_runtime().spawn(async move {
                                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                                ctx_clone.send_viewport_cmd(egui::ViewportCommand::Close);
                            });
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
                ui.label(RichText::new("PolyWatcher").strong().size(20.0).color(Color32::from_rgb(0, 255, 255)));
                ui.separator();
                ui.label(format!("Wallet: {}", if self.wallet_address.is_empty() { "⚠️ Configuração necessária" } else { &self.wallet_address }));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(format!("Balance: ${:.2} USDC", self.balance)).strong().color(Color32::GREEN));
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
                egui::Grid::new("ladder").striped(true).show(ui, |ui| {
                    ui.label("Back (Bid)");
                    ui.label("ODDS");
                    ui.label("Lay (Ask)");
                    ui.end_row();

                    for price_cent in (1..100).rev() {
                        let price = price_cent as f64 / 100.0;
                        let odds = if price > 0.0 { 1.0 / price } else { 0.0 };
                        
                        // Bid
                        let bid_size = book.bids.get(&price_cent).cloned().unwrap_or_default();
                        if ui.add(egui::Button::new(RichText::new(&bid_size).color(Color32::BLACK)).fill(Color32::from_rgb(173, 216, 230))).clicked() {
                            self.place_order(Side::BUY, price);
                        }

                        ui.label(RichText::new(format!("{:.2} ({}¢)", odds, price_cent)).strong());

                        // Ask
                        let ask_size = book.asks.get(&price_cent).cloned().unwrap_or_default();
                        if ui.add(egui::Button::new(RichText::new(&ask_size).color(Color32::BLACK)).fill(Color32::from_rgb(255, 182, 193))).clicked() {
                            self.place_order(Side::SELL, price);
                        }
                        ui.end_row();
                    }
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
