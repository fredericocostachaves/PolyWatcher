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
    UsdcBalance(f64),
    TokenBalance(f64),
    OpenOrders(Vec<OrderSummary>),
    Error(String),
}

#[derive(Clone, Debug)]
pub struct OrderSummary {
    pub price: f64,
    pub size: f64,
    pub side: Side,
    pub token_id: String,
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

        auth_builder = auth_builder.signature_type(SignatureType::Proxy);
        if !self.poly_funder_address.is_empty() {
            if let Ok(funder_addr) = alloy::primitives::Address::from_str(&self.poly_funder_address) {
                auth_builder = auth_builder.funder(funder_addr);
            } else {
                return Err("Endereço do funder inválido".to_string());
            }
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
    usdc_balance: f64,
    selected_token_balance: f64,
    open_orders: Vec<OrderSummary>,
    receiver: mpsc::Receiver<AppMessage>,
    sender: mpsc::Sender<AppMessage>,
    clob: Option<ClobClient>,
    pub logout_requested: bool,
    maximize_pending: bool,
    last_data_refresh: std::time::Instant,
    monitor_handle: Option<tokio::task::JoinHandle<()>>,
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
            stake: "1".to_string(),
            status_log: vec!["Sistema iniciado".to_string()],
            usdc_balance: 0.0,
            selected_token_balance: 0.0,
            open_orders: Vec::new(),
            receiver: rx,
            sender: tx.clone(),
            clob,
            logout_requested: false,
            maximize_pending: true,
            last_data_refresh: std::time::Instant::now(),
            monitor_handle: None,
        };

        app.load_initial_data();
        if initial_total_value.is_none() {
            app.refresh_total_value();
        }
        app.refresh_usdc_balance();

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

    fn refresh_usdc_balance(&mut self) {
        if let Some(clob) = &self.clob {
            let clob = clob.clone();
            let tx = self.sender.clone();
            get_runtime().spawn(async move {
                match clob.get_usdc_balance().await {
                    Ok(v) => { let _ = tx.send(AppMessage::UsdcBalance(v)).await; }
                    Err(e) => { let _ = tx.send(AppMessage::Error(format!("Erro ao buscar saldo USDC: {}", e))).await; }
                }
            });
        }
    }

    fn refresh_open_orders(&mut self) {
        if let Some(clob) = &self.clob {
            let clob = clob.clone();
            let tx = self.sender.clone();
            get_runtime().spawn(async move {
                match clob.get_open_orders().await {
                    Ok(orders) => { let _ = tx.send(AppMessage::OpenOrders(orders)).await; }
                    Err(e) => { let _ = tx.send(AppMessage::Error(format!("Erro ao buscar ordens: {}", e))).await; }
                }
            });
        }
    }

    fn refresh_token_balance(&mut self) {
        if let (Some(clob), Some(tid)) = (&self.clob, &self.selected_token_id) {
            let clob = clob.clone();
            let tid = tid.clone();
            let tx = self.sender.clone();
            get_runtime().spawn(async move {
                match clob.get_token_balance(&tid).await {
                    Ok(v) => { let _ = tx.send(AppMessage::TokenBalance(v)).await; }
                    Err(e) => { let _ = tx.send(AppMessage::Error(format!("Erro ao buscar saldo de token: {}", e))).await; }
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
        let stake_val = self.stake.parse::<f64>().unwrap_or(10.0);
        
        // O stakeholder geralmente define quanto quer apostar em USDC.
        // O SDK espera 'size' em shares. No Polymarket, 1 share = $1 se vencer.
        // Logo, para apostar 'stake' dólares ao preço 'price', precisamos de 'stake / price' shares.
        let size = if price > 0.0 { stake_val / price } else { stake_val };
        
        let tx = self.sender.clone();
        let clob = clob.clone();

        get_runtime().spawn(async move {
            match clob.post_order(token_id, side, price, size).await {
                Ok(resp) => { let _ = tx.send(AppMessage::Error(format!("Ordem enviada: {}", resp))).await; }
                Err(e) => { let _ = tx.send(AppMessage::Error(format!("Erro na ordem: {}", e))).await; }
            }
        });
    }

    fn place_market_order(&mut self, side: Side) {
        let Some(clob) = self.clob.as_ref() else {
            self.status_log.push("Erro: Clob não configurado".to_string());
            return;
        };
        let Some(token_id) = self.selected_token_id.clone() else { return; };
        let size = self.stake.parse::<f64>().unwrap_or(10.0);
        let tx = self.sender.clone();
        let clob = clob.clone();

        get_runtime().spawn(async move {
            match clob.post_market_order(token_id, side, size).await {
                Ok(resp) => { let _ = tx.send(AppMessage::Error(format!("Ordem a mercado enviada: {}", resp))).await; }
                Err(e) => { let _ = tx.send(AppMessage::Error(format!("Erro na ordem a mercado: {}", e))).await; }
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
        if self.selected_token_id.as_ref() == Some(&token_id) {
            return;
        }

        self.selected_token_id = Some(token_id.clone());
        self.orderbook = Orderbook::default(); // Limpa o book ao trocar de token
        self.selected_token_balance = 0.0;      // Limpa o saldo ao trocar de token
        self.refresh_token_balance();          // Busca o saldo do novo token imediatamente
        self.refresh_open_orders();            // Atualiza ordens imediatamente

        if let Some(handle) = self.monitor_handle.take() {
            handle.abort();
        }

        let sender = self.sender.clone();
        let tx_err = self.sender.clone();
        let tid = token_id.clone();
        self.monitor_handle = Some(get_runtime().spawn(async move {
            if let Err(e) = crate::watcher::monitor_token_egui(&tid, sender).await {
                let _ = tx_err.send(AppMessage::Error(format!("Erro no monitor do book: {}", e))).await;
                eprintln!("Monitor error for {}: {}", tid, e);
            }
        }));
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

    fn event_matches_search(&self, event: &GammaEvent, query: &str) -> bool {
        if query.is_empty() { return true; }
        let query = query.to_lowercase();
        
        // 1. Busca no título do evento
        if let Some(title) = &event.title {
            if title.to_lowercase().contains(&query) { return true; }
        }
        
        // 2. Busca no slug (contém nomes de times/liga formatados)
        if let Some(slug) = &event.slug {
            if slug.to_lowercase().contains(&query) { return true; }
        }
        
        // 3. Busca nas tags associadas (ligas, esportes)
        if let Some(tags) = &event.tags {
            for tag in tags {
                if let Some(label) = self.tags.get(&tag.id) {
                    if label.to_lowercase().contains(&query) { return true; }
                }
            }
        }
        
        // 4. Busca nos mercados (pergunta)
        if let Some(markets) = &event.markets {
            for m in markets {
                if let Some(q) = &m.question {
                    if q.to_lowercase().contains(&query) { return true; }
                }
            }
        }

        false
    }

    fn update_impl(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.last_data_refresh.elapsed().as_secs() >= 10 {
            self.refresh_usdc_balance();
            self.refresh_token_balance();
            self.refresh_open_orders();
            self.last_data_refresh = std::time::Instant::now();
        }

        if self.maximize_pending {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            self.maximize_pending = false;
        }

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
                AppMessage::UsdcBalance(v) => self.usdc_balance = v,
                AppMessage::TokenBalance(v) => self.selected_token_balance = v,
                AppMessage::OpenOrders(orders) => self.open_orders = orders,
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
                    let mut found_any = false;
                    
                    for event in &self.events {
                        if self.event_matches_search(event, &self.search_global) {
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
                            let match_search = self.event_matches_search(event, &self.search_global);
                            
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
            if let Some(event) = self.selected_event.clone() {
                // Top Header: Event Title and USDC Balance
                ui.horizontal(|ui| {
                    ui.heading(RichText::new(event.title.as_deref().unwrap_or("Untitled")).color(Color32::WHITE).size(18.0));
                    
                    // Display Price and Spread logic from documentation
                    let best_bid = self.orderbook.bids.keys().max().copied();
                    let best_ask = self.orderbook.asks.keys().min().copied();
                    
                    if let (Some(bid), Some(ask)) = (best_bid, best_ask) {
                        let bid_f = bid as f64 / 100.0;
                        let ask_f = ask as f64 / 100.0;
                        let spread = ask_f - bid_f;
                        let midpoint = (bid_f + ask_f) / 2.0;
                        
                        // "The displayed price is the midpoint of the bid-ask spread. 
                        // If the spread is wider than $0.10, the last traded price is shown instead."
                        let display_price = if spread > 0.10 {
                            self.orderbook.last_price.unwrap_or(midpoint)
                        } else {
                            midpoint
                        };
                        
                        ui.add_space(20.0);
                        ui.label(RichText::new(format!("Price: {:.2} ({:.0}%)", display_price, display_price * 100.0))
                            .strong().color(Color32::YELLOW).size(16.0));
                        ui.add_space(10.0);
                        ui.label(RichText::new(format!("Spread: {:.2}", spread))
                            .small().color(if spread > 0.10 { Color32::RED } else { Color32::GRAY }));
                    } else if let Some(last) = self.orderbook.last_price {
                        ui.add_space(20.0);
                        ui.label(RichText::new(format!("Last: {:.2} ({:.0}%)", last, last * 100.0))
                            .strong().color(Color32::YELLOW).size(16.0));
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{:.2} USDC", self.usdc_balance)).color(Color32::from_rgb(165, 214, 167)).strong().size(16.0));
                            if self.selected_token_id.is_some() {
                                ui.separator();
                                ui.label(RichText::new(format!("Pos: {:.2} shares", self.selected_token_balance)).color(Color32::from_rgb(100, 181, 246)).strong().size(16.0));
                            }
                        });
                    });
                });
                ui.separator();
                
                // Non-scrolling top part: Markets and Stake
                if let Some(markets) = &event.markets {
                    for market in markets {
                        ui.label(RichText::new(market.question.as_deref().unwrap_or("Mercado")).color(Color32::YELLOW).small());
                        ui.horizontal(|ui| {
                            if let (Some(outcomes), Some(tokens)) = (&market.outcomes, &market.clob_token_ids) {
                                for (outcome, token_id) in outcomes.iter().zip(tokens.iter()) {
                                    let tid_str = token_id.to_string();
                                    let is_selected = self.selected_token_id.as_ref() == Some(&tid_str);
                                    if ui.selectable_label(is_selected, RichText::new(outcome).color(Color32::WHITE).strong()).clicked() {
                                        clicked_token = Some(tid_str);
                                    }
                                }
                            }
                        });
                        ui.add_space(4.0);
                    }
                }
                
                ui.add_space(8.0);
                
                // Stake Buttons
                ui.horizontal(|ui| {
                    for amt in ["10", "25", "50", "100", "200", "500"] {
                        let btn = egui::Button::new(RichText::new(amt).color(Color32::WHITE).strong())
                            .fill(Color32::from_rgb(66, 66, 66))
                            .min_size(egui::vec2(40.0, 24.0));
                        if ui.add(btn).clicked() {
                            self.stake = amt.to_string();
                        }
                    }
                    ui.add_space(20.0);
                    ui.label(RichText::new("Bet: $").color(Color32::WHITE).strong());
                    ui.add(egui::TextEdit::singleline(&mut self.stake).desired_width(80.0));
                    
                    ui.add_space(20.0);
                    if ui.add(egui::Button::new(RichText::new("BUY MARKET").color(Color32::WHITE).strong())
                        .fill(Color32::from_rgb(21, 101, 192))).clicked() {
                        self.place_market_order(Side::BUY);
                    }
                    if ui.add(egui::Button::new(RichText::new("SELL MARKET").color(Color32::WHITE).strong())
                        .fill(Color32::from_rgb(183, 28, 28))).clicked() {
                        self.place_market_order(Side::SELL);
                    }
                });

                ui.separator();

                let avail_h = ui.available_height();
                let book_h = (avail_h * 0.70).max(200.0);
                let orders_h = (avail_h - book_h - 20.0).max(100.0);

                // Book (Ladder) with its own ScrollArea
                ui.label(RichText::new("ORDER BOOK (Ladder)").strong().color(Color32::WHITE));
                egui::ScrollArea::vertical()
                    .id_salt("book_scroll")
                    .max_height(book_h)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let book = self.orderbook.clone();
                        let stake_val: f64 = self.stake.parse().unwrap_or(0.0);

                        egui::Grid::new("ladder")
                            .spacing([0.0, 0.0])
                            .min_col_width(60.0)
                            .show(ui, |ui| {
                            // Headers
                            ui.label(RichText::new("P/L").small().color(Color32::GRAY));
                            ui.vertical_centered(|ui| ui.label(RichText::new("Back").strong().color(Color32::from_rgb(33, 150, 243))));
                            ui.vertical_centered(|ui| ui.label(RichText::new("ODDS").strong().color(Color32::WHITE)));
                            ui.vertical_centered(|ui| ui.label(RichText::new("Lay").strong().color(Color32::from_rgb(244, 67, 54))));
                            ui.label(RichText::new("CON").small().color(Color32::GRAY));
                            ui.end_row();

                            for price_cent in 1..100 {
                                let price = price_cent as f64 / 100.0;
                                let odds = if price > 0.0 { 1.0 / price } else { 0.0 };
                                
                                let bid_size = book.bids.get(&price_cent).cloned().unwrap_or_default();
                                let ask_size = book.asks.get(&price_cent).cloned().unwrap_or_default();

                                let my_orders = self.open_orders.iter()
                                    .filter(|o| (o.price * 100.0).round() as i32 == price_cent && Some(&o.token_id) == self.selected_token_id.as_ref())
                                    .collect::<Vec<_>>();
                                
                                let is_my_bid = my_orders.iter().any(|o| o.side == Side::BUY);
                                let is_my_ask = my_orders.iter().any(|o| o.side == Side::SELL);
                                let my_bid_size: f64 = my_orders.iter().filter(|o| o.side == Side::BUY).map(|o| o.size).sum();
                                let my_ask_size: f64 = my_orders.iter().filter(|o| o.side == Side::SELL).map(|o| o.size).sum();

                                // P/L calculation based on liquidity
                                let bid_size_shares = bid_size.parse::<f64>().unwrap_or(0.0);
                                let ask_size_shares = ask_size.parse::<f64>().unwrap_or(0.0);
                                
                                let pl_val = if ask_size_shares > 0.0 {
                                    // Back profit if matched against Asks
                                    let matched_shares = (stake_val / price).min(ask_size_shares);
                                    matched_shares * (1.0 - price)
                                } else if bid_size_shares > 0.0 {
                                    // Lay profit if matched against Bids
                                    let matched_shares = (stake_val / price).min(bid_size_shares);
                                    matched_shares * price
                                } else {
                                    0.0
                                };
                                let pl_color = if pl_val > 0.0 { Color32::from_rgb(76, 175, 80) } else { Color32::GRAY };
                                
                                // P/L Column
                                ui.add_sized([50.0, 22.0], egui::Label::new(RichText::new(format!("{:+2.2}", pl_val)).color(pl_color).small()));

                                // Back Column (Bids)
                                let mut bid_label = bid_size.clone();
                                if is_my_bid {
                                    bid_label = format!("${:.2} {}", my_bid_size, bid_label);
                                }
                                let bid_btn = egui::Button::new(RichText::new(&bid_label).strong().color(if is_my_bid { Color32::YELLOW } else { Color32::WHITE }))
                                    .fill(if is_my_bid { Color32::from_rgb(27, 94, 32) } else if bid_size.is_empty() { Color32::from_rgb(25, 27, 31) } else { Color32::from_rgb(21, 101, 192) })
                                    .corner_radius(0.0)
                                    .min_size(egui::vec2(80.0, 22.0));
                                if ui.add(bid_btn).clicked() {
                                    self.place_order(Side::BUY, price);
                                }

                                // ODDS Column
                                let odds_text = format!("{:.2} / {}¢", odds, price_cent);
                                let odds_btn = egui::Button::new(RichText::new(odds_text).strong().color(Color32::WHITE))
                                    .fill(Color32::from_rgb(45, 45, 45))
                                    .corner_radius(0.0)
                                    .min_size(egui::vec2(100.0, 22.0));
                                ui.add(odds_btn);

                                // Lay Column (Asks)
                                let mut ask_label = ask_size.clone();
                                if is_my_ask {
                                    ask_label = format!("{} ${:.2}", ask_label, my_ask_size);
                                }
                                let ask_btn = egui::Button::new(RichText::new(&ask_label).strong().color(if is_my_ask { Color32::YELLOW } else { Color32::WHITE }))
                                    .fill(if is_my_ask { Color32::from_rgb(183, 28, 28) } else if ask_size.is_empty() { Color32::from_rgb(25, 27, 31) } else { Color32::from_rgb(183, 28, 28) })
                                    .corner_radius(0.0)
                                    .min_size(egui::vec2(80.0, 22.0));
                                if ui.add(ask_btn).clicked() {
                                    self.place_order(Side::SELL, price);
                                }
                                
                                // CON Column placeholder
                                ui.label("");

                                ui.end_row();
                            }
                        });
                    });

                ui.separator();
                
                // Open Orders with its own ScrollArea
                ui.label(RichText::new("OPEN ORDERS (Sent)").strong().color(Color32::WHITE));

                let (buy_orders, sell_orders): (Vec<_>, Vec<_>) = self.open_orders.iter()
                    .filter(|o| Some(&o.token_id) == self.selected_token_id.as_ref())
                    .partition(|o| o.side == Side::BUY);

                egui::ScrollArea::vertical()
                    .id_salt("orders_scroll")
                    .max_height(orders_h)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.columns(2, |columns| {
                            columns[0].vertical(|ui| {
                                ui.label(RichText::new("Buy Orders").strong().color(Color32::from_rgb(33, 150, 243)));
                                ui.separator();
                                
                                if buy_orders.is_empty() {
                                    ui.label(RichText::new("Nenhuma ordem ativa").small().italics().color(Color32::GRAY));
                                } else {
                                    for o in buy_orders {
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new(format!("{:.2} / {}¢", 1.0/o.price, (o.price*100.0).round() as i32)).color(Color32::from_rgb(100, 181, 246)));
                                            ui.label(RichText::new(format!("${:.2}", o.size)).color(Color32::WHITE));
                                            if ui.button(RichText::new("X").color(Color32::RED)).clicked() {
                                                // self.cancel_order(&o.order_id);
                                            }
                                        });
                                    }
                                }
                            });
                            columns[1].vertical(|ui| {
                                ui.label(RichText::new("Sell Orders").strong().color(Color32::from_rgb(244, 67, 54)));
                                ui.separator();
                                
                                if sell_orders.is_empty() {
                                    ui.label(RichText::new("Nenhuma ordem ativa").small().italics().color(Color32::GRAY));
                                } else {
                                    for o in sell_orders {
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new(format!("{:.2} / {}¢", 1.0/o.price, (o.price*100.0).round() as i32)).color(Color32::from_rgb(255, 128, 128)));
                                            ui.label(RichText::new(format!("${:.2}", o.size)).color(Color32::WHITE));
                                            if ui.button(RichText::new("X").color(Color32::RED)).clicked() {
                                                // self.cancel_order(&o.order_id);
                                            }
                                        });
                                    }
                                }
                            });
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
