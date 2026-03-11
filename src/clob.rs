use alloy::signers::local::PrivateKeySigner;
use alloy::signers::Signer;
use alloy_primitives::{Address, U256};
use polymarket_client_sdk::clob::types::Side as SdkSide;
use polymarket_client_sdk::clob::{Client as SdkClient, Config as SdkConfig};
use polymarket_client_sdk::types::Decimal;
use polymarket_client_sdk::POLYGON;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionValue {
    pub user: String,
    pub value: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub address: String,
    pub api_key: String,
    pub api_secret: String,
    pub passphrase: String,
    pub private_key: String,
    pub funder_address: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum Side {
    BUY,
    SELL,
}

#[derive(Clone)]
pub struct ClobClient {
    pub creds: Credentials,
}

impl ClobClient {
    pub fn new(creds: Credentials) -> Self {
        Self { creds }
    }

    pub fn from_env() -> Result<Self, String> {
        let private_key = std::env::var("POLY_PRIVATE_KEY")
            .map_err(|_| "POLY_PRIVATE_KEY not found in environment".to_string())?;
        let api_key = std::env::var("POLY_API_KEY")
            .map_err(|_| "POLY_API_KEY not found in environment".to_string())?;
        let api_secret = std::env::var("POLY_API_SECRET")
            .map_err(|_| "POLY_API_SECRET not found in environment".to_string())?;
        let passphrase = std::env::var("POLY_PASSPHRASE")
            .map_err(|_| "POLY_PASSPHRASE not found in environment".to_string())?;
        let funder_address = std::env::var("POLY_FUNDER_ADDRESS")
            .ok()
            .filter(|s| !s.is_empty());

        // Deriva o endereço a partir da private key
        let signer = PrivateKeySigner::from_str(&private_key)
            .map_err(|e| format!("Invalid private key: {}", e))?;
        
        // Se tiver funder, o endereço alvo para saldo/exibição deve ser o funder
        let address = if let Some(funder) = &funder_address {
            funder.to_lowercase()
        } else {
            format!("{:#x}", signer.address()).to_lowercase()
        };

        let api_key_tail = if api_key.len() > 4 { &api_key[api_key.len()-4..] } else { &api_key };
        eprintln!("📝 Carregando credenciais do .env:");
        eprintln!("  - Address (Alvo): {}", address);
        eprintln!("  - API Key: ...{}", api_key_tail);
        if let Some(f) = &funder_address {
             eprintln!("  - Funder: {}", f);
        } else {
             eprintln!("  - Funder: None (EOA)");
        }

        Ok(Self::new(Credentials {
            address,
            api_key,
            api_secret,
            passphrase,
            private_key,
            funder_address,
        }))
    }

    async fn get_sdk_client(
        &self,
    ) -> Result<
        SdkClient<
            polymarket_client_sdk::auth::state::Authenticated<polymarket_client_sdk::auth::Normal>,
        >,
        String,
    > {
        use polymarket_client_sdk::auth::Credentials as SdkCredentials;
        use polymarket_client_sdk::clob::types::SignatureType;

        let signer = PrivateKeySigner::from_str(&self.creds.private_key)
            .map_err(|e| format!("Erro no Signer: {}", e))?
            .with_chain_id(Some(POLYGON));

        eprintln!("🔑 Endereço Signer (EOA): {:?}", signer.address());

        // Parse o API key como UUID
        let api_key_uuid = Uuid::parse_str(&self.creds.api_key)
            .map_err(|e| format!("API Key inválida (UUID): {}", e))?;

        eprintln!("🔑 Inicializando SDK Client para: {}", self.creds.address);
        if let Some(ref funder) = self.creds.funder_address {
            eprintln!("🏦 Wallet Alvo (Proxy/Safe): {}", funder);
        } else {
            eprintln!("👤 Wallet Alvo (EOA): {}", signer.address());
        }

        // Cria as credenciais do SDK
        let sdk_creds = SdkCredentials::new(
            api_key_uuid,
            self.creds.api_secret.clone(),
            self.creds.passphrase.clone(),
        );

        let mut auth_builder = SdkClient::new("https://clob.polymarket.com", SdkConfig::default())
            .map_err(|e| format!("Erro ao criar cliente: {}", e))?
            .authentication_builder(&signer)
            .credentials(sdk_creds);

        // Configura funder e signature type SE fornecido
        if let Some(funder) = self.creds.funder_address.as_ref().filter(|s| !s.is_empty()) {
            if let Ok(addr) = Address::from_str(funder) {
                eprintln!("🏦 Usando Gnosis Safe - Funder: {:?}", addr);
                auth_builder = auth_builder
                    .funder(addr)
                    .signature_type(SignatureType::GnosisSafe);
            } else {
                eprintln!("⚠️ Funder address inválido: {}, usando EOA", funder);
                auth_builder = auth_builder.signature_type(SignatureType::Eoa);
            }
        } else {
            // Se NÃO tem funder, o SDK exige SignatureType::Eoa explicitamente ou None
            eprintln!("👤 Usando EOA (sem funder)");
            auth_builder = auth_builder.signature_type(SignatureType::Eoa);
        }

        eprintln!("🔐 Autenticando...");
        let client = auth_builder
            .authenticate()
            .await
            .map_err(|e| format!("Erro de Autenticação: {}", e))?;

        eprintln!("✅ Cliente SDK pronto.");

        Ok(client)
    }

    pub async fn post_order(
        &self,
        token_id: String,
        side: Side,
        price: f64,
        size: f64,
    ) -> Result<String, String> {
        let client = self.get_sdk_client().await?;

        let sdk_side = match side {
            Side::BUY => SdkSide::Buy,
            Side::SELL => SdkSide::Sell,
        };

        // Use Decimal for precise financial calculations
        let price_dec = Decimal::from_f64_retain(price).ok_or("Invalid price")?;
        let size_dec = Decimal::from_f64_retain(size).ok_or("Invalid size")?;

        let signer = PrivateKeySigner::from_str(&self.creds.private_key)
            .map_err(|e| format!("Signer error: {}", e))?
            .with_chain_id(Some(POLYGON));

        let order = client
            .limit_order()
            .token_id(U256::from_str(&token_id).map_err(|e| e.to_string())?)
            .side(sdk_side)
            .price(price_dec)
            .size(size_dec)
            .build()
            .await
            .map_err(|e| format!("Order build error: {}", e))?;

        let signed_order = client
            .sign(&signer, order)
            .await
            .map_err(|e| format!("Signing error: {}", e))?;

        let resp = client
            .post_order(signed_order)
            .await
            .map_err(|e| format!("Post order error: {}", e))?;

        Ok(format!("{:?}", resp))
    }

    pub async fn get_total_value(&self) -> Result<f64, String> {
        let client = reqwest::Client::new();
        let url = format!("https://data-api.polymarket.com/value?user={}", self.creds.address.to_lowercase());
        
        eprintln!("🔍 Consultando Valor Total para: {}", self.creds.address.to_lowercase());
        
        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Erro ao chamar API de dados: {}", e))?;
            
        let text = resp.text().await.map_err(|e| format!("Erro ao obter corpo da resposta: {}", e))?;
        eprintln!("🔍 Resposta da API de Dados: {}", text);
        
        let resp_json: Vec<PositionValue> = serde_json::from_str(&text)
            .map_err(|e| format!("Erro ao processar JSON da API de dados: {}", e))?;

        let total = resp_json.first().map(|v| v.value).unwrap_or(0.0);
        eprintln!("✅ Valor Total obtido: {}", total);
        
        Ok(total)
    }
}
