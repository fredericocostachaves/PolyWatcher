use alloy::signers::local::PrivateKeySigner;
use alloy::signers::Signer;
use alloy_primitives::{Address, U256};
use polymarket_client_sdk::clob::types::{Side as SdkSide, AssetType, SignatureType, Amount};
use polymarket_client_sdk::clob::types::request::{BalanceAllowanceRequest, OrdersRequest};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

    pub fn get_signer(&self) -> Result<PrivateKeySigner, String> {
        PrivateKeySigner::from_str(&self.creds.private_key)
            .map_err(|e| format!("Erro no Signer: {}", e))
            .map(|s| s.with_chain_id(Some(POLYGON)))
    }

    pub fn get_signature_type(&self) -> SignatureType {
        SignatureType::Proxy
    }

    pub fn from_env() -> Result<Self, String> {
        let private_key = std::env::var("POLY_PRIVATE_KEY")
            .map_err(|_| "POLY_PRIVATE_KEY não encontrada".to_string())?
            .trim()
            .to_string();
        let api_key = std::env::var("POLY_API_KEY")
            .map_err(|_| "POLY_API_KEY não encontrada".to_string())?
            .trim()
            .to_string();
        let api_secret = std::env::var("POLY_API_SECRET")
            .map_err(|_| "POLY_API_SECRET não encontrada".to_string())?
            .trim()
            .to_string();
        let passphrase = std::env::var("POLY_PASSPHRASE")
            .map_err(|_| "POLY_PASSPHRASE não encontrada".to_string())?
            .trim()
            .to_string();
        let funder_address = std::env::var("POLY_FUNDER_ADDRESS")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_lowercase());

        let signer = PrivateKeySigner::from_str(&private_key)
            .map_err(|e| format!("Erro na Private Key: {}", e))?;

        let address = if let Some(ref funder) = funder_address {
            funder.clone()
        } else {
            format!("{:#x}", signer.address()).to_lowercase()
        };

        let api_key_tail = if api_key.len() > 4 { &api_key[api_key.len()-4..] } else { &api_key };
        eprintln!("📝 Credenciais carregadas:");
        eprintln!("  - Address: {}", address);
        eprintln!("  - API Key: ...{}", api_key_tail);
        eprintln!("  - Funder: {:?}", funder_address);

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

        let signer = self.get_signer()?;

        // Parse o API key como UUID
        let api_key_uuid = Uuid::parse_str(&self.creds.api_key)
            .map_err(|e| format!("API Key inválida (UUID): {}", e))?;

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

        // Configura funder e signature type
        let sig_type = self.get_signature_type();
        auth_builder = auth_builder.signature_type(sig_type);
        
        if sig_type != SignatureType::Eoa {
            if let Some(funder) = self.creds.funder_address.as_ref().filter(|s| !s.is_empty()) {
                if let Ok(addr) = Address::from_str(funder) {
                    auth_builder = auth_builder.funder(addr);
                }
            }
        }

        let client = auth_builder
            .authenticate()
            .await
            .map_err(|e| format!("Erro de Autenticação: {}", e))?;

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
        let signer = self.get_signer()?;

        let sdk_side = match side {
            Side::BUY => SdkSide::Buy,
            Side::SELL => SdkSide::Sell,
        };

        let price_dec = Decimal::from_f64_retain(price)
            .ok_or("Invalid price")?
            .round_dp(2);
        let size_dec = Decimal::from_f64_retain(size)
            .ok_or("Invalid size")?
            .round_dp(4);

        let tid = U256::from_str(&token_id).map_err(|e| e.to_string())?;

        let order = client
            .limit_order()
            .token_id(tid)
            .side(sdk_side)
            .price(price_dec)
            .size(size_dec)
            .build()
            .await
            .map_err(|e| format!("Order build error: {}", e))?;


        // Logs para depuração de assinatura
        eprintln!("📝 Ordem construída:");
        eprintln!("  - Maker: {:?}", order.order.maker);
        eprintln!("  - Signer (na ordem): {:?}", order.order.signer);
        eprintln!("  - Signature Type: {:?}", order.order.signatureType);
        eprintln!("  - Signer (EOA): {:?}", signer.address());

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

    pub async fn post_market_order(
        &self,
        token_id: String,
        side: Side,
        size: f64,
    ) -> Result<String, String> {
        let client = self.get_sdk_client().await?;
        let signer = self.get_signer()?;

        let sdk_side = match side {
            Side::BUY => SdkSide::Buy,
            Side::SELL => SdkSide::Sell,
        };

        let size_dec = Decimal::from_f64_retain(size)
            .ok_or("Invalid size")?
            .round_dp(6);

        let tid = U256::from_str(&token_id).map_err(|e| e.to_string())?;

        let order = client
            .market_order()
            .token_id(tid)
            .side(sdk_side)
            .amount(Amount::usdc(size_dec).map_err(|e| e.to_string())?)
            .build()
            .await
            .map_err(|e| format!("Market order build error: {}", e))?;

        // Logs para depuração de assinatura
        eprintln!("📝 Market Order construída:");
        eprintln!("  - Maker: {:?}", order.order.maker);
        eprintln!("  - Signer (na ordem): {:?}", order.order.signer);
        eprintln!("  - Signature Type: {:?}", order.order.signatureType);
        eprintln!("  - Signer (EOA): {:?}", signer.address());

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

    pub async fn get_usdc_balance(&self) -> Result<f64, String> {
        use rust_decimal::prelude::ToPrimitive;

        let client = self.get_sdk_client().await?;
        
        let sig_type = self.get_signature_type();

        let request = BalanceAllowanceRequest::builder()
            .asset_type(AssetType::Collateral)
            .signature_type(sig_type)
            .build();

        let resp = client.balance_allowance(request)
            .await
            .map_err(|e| format!("Erro ao buscar saldo via SDK: {}", e))?;
        
        Ok(resp.balance.to_f64().unwrap_or(0.0))
    }

    pub async fn get_token_balance(&self, token_id: &str) -> Result<f64, String> {
        use rust_decimal::prelude::ToPrimitive;

        let client = self.get_sdk_client().await?;
        
        let sig_type = self.get_signature_type();

        let tid = U256::from_str(token_id)
            .map_err(|e| format!("Token ID inválido: {}", e))?;

        let request = BalanceAllowanceRequest::builder()
            .asset_type(AssetType::Conditional)
            .token_id(tid)
            .signature_type(sig_type)
            .build();

        let resp = client.balance_allowance(request)
            .await
            .map_err(|e| format!("Erro ao buscar saldo de token: {}", e))?;
        
        Ok(resp.balance.to_f64().unwrap_or(0.0))
    }

    pub async fn get_open_orders(&self) -> Result<Vec<crate::ui::OrderSummary>, String> {
        use rust_decimal::prelude::ToPrimitive;

        let client = self.get_sdk_client().await?;
        
        let request = OrdersRequest::builder().build();
        
        let resp = client.orders(&request, None)
            .await
            .map_err(|e| format!("Erro ao buscar ordens abertas: {}", e))?;
        
        let summaries = resp.data.into_iter().map(|o| {
            crate::ui::OrderSummary {
                price: o.price.to_f64().unwrap_or(0.0),
                size: (o.original_size - o.size_matched).to_f64().unwrap_or(0.0),
                side: match o.side {
                    SdkSide::Buy => Side::BUY,
                    SdkSide::Sell => Side::SELL,
                    _ => Side::BUY,
                },
                token_id: o.asset_id.to_string(),
            }
        }).collect::<Vec<_>>();
        
        Ok(summaries)
    }
}
