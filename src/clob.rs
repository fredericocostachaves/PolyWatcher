#![allow(dead_code)]
use alloy::primitives::{Address, U256};
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::Signer;
use polymarket_client_sdk::clob::types::Side as SdkSide;
use polymarket_client_sdk::clob::{Client as SdkClient, Config as SdkConfig};
use polymarket_client_sdk::types::Decimal;
use polymarket_client_sdk::POLYGON;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use uuid::Uuid;

pub const COLLATERAL_DECIMALS: u8 = 6;

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
        let funder_address = std::env::var("POLY_FUNDER_ADDRESS").ok();

        // Deriva o endereço a partir da private key
        let signer = PrivateKeySigner::from_str(&private_key)
            .map_err(|e| format!("Invalid private key: {}", e))?;
        let address = format!("{:?}", signer.address());

        eprintln!("📝 Carregando credenciais do .env:");
        eprintln!("  - Address (EOA): {}", address);
        eprintln!("  - API Key: {}", api_key);
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

    async fn get_sdk_client(&self) -> Result<SdkClient<polymarket_client_sdk::auth::state::Authenticated<polymarket_client_sdk::auth::Normal>>, String> {
        use polymarket_client_sdk::clob::types::SignatureType;
        use polymarket_client_sdk::auth::Credentials as SdkCredentials;

        let signer = PrivateKeySigner::from_str(&self.creds.private_key)
            .map_err(|e| format!("Signer error: {}", e))?
            .with_chain_id(Some(POLYGON));

        eprintln!("🔑 Signer address (EOA): {:?}", signer.address());

        // Parse o API key como UUID
        let api_key_uuid = Uuid::parse_str(&self.creds.api_key)
            .map_err(|e| format!("Invalid API Key UUID: {}", e))?;

        // Cria as credenciais do SDK
        let sdk_creds = SdkCredentials::new(
            api_key_uuid,
            self.creds.api_secret.clone(),
            self.creds.passphrase.clone(),
        );

        eprintln!("🔐 Usando credenciais fornecidas (não derivando novas)");

        let mut auth_builder = SdkClient::new("https://clob.polymarket.com", SdkConfig::default())
            .map_err(|e| format!("Client creation error: {}", e))?
            .authentication_builder(&signer)
            .credentials(sdk_creds);

        // Configura funder e signature type se fornecido
        if let Some(funder) = &self.creds.funder_address {
            if let Ok(addr) = Address::from_str(funder) {
                eprintln!("🏦 Usando Gnosis Safe - Funder: {:?}", addr);
                auth_builder = auth_builder
                    .funder(addr)
                    .signature_type(SignatureType::GnosisSafe);
            }
        } else {
            eprintln!("👤 Usando EOA (sem funder)");
            auth_builder = auth_builder.signature_type(SignatureType::Eoa);
        }

        eprintln!("🔐 Autenticando com Polymarket...");
        let client = auth_builder.authenticate()
            .await
            .map_err(|e| format!("Authentication error: {}", e))?;

        eprintln!("✅ Autenticação bem-sucedida!");

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

        let signed_order = client.sign(&signer, order).await
            .map_err(|e| format!("Signing error: {}", e))?;

        let resp = client.post_order(signed_order).await
            .map_err(|e| format!("Post order error: {}", e))?;

        Ok(format!("{:?}", resp))
    }

    pub async fn get_balance(&self) -> Result<f64, String> {
        use polymarket_client_sdk::clob::types::AssetType;
        use polymarket_client_sdk::clob::types::request::BalanceAllowanceRequest;
        use rust_decimal::prelude::ToPrimitive;

        let client = self.get_sdk_client().await?;

        eprintln!("🔍 Cliente autenticado criado!");
        eprintln!("🔍 Funder configurado: {:?}", self.creds.funder_address);

        // Cria a requisição APENAS com asset_type
        // O SDK já sabe qual signature type usar baseado na configuração do cliente
        let request = BalanceAllowanceRequest::builder()
            .asset_type(AssetType::Collateral)
            .build();

        eprintln!("🔍 Fazendo requisição de balance autenticada...");

        let balances = client.balance_allowance(request)
            .await
            .map_err(|e| format!("Balance error: {}", e))?;

        eprintln!("🔍 Balance response recebido!");
        eprintln!("🔍 Balance: {:?}", balances.balance);

        // O balance já vem como Decimal
        let balance_dec = balances.balance;

        // Converte para f64 (já está em unidades de USDC com 6 decimais)
        let raw_val = balance_dec.to_f64().unwrap_or(0.0);

        // Divide por 1 milhão para converter de micro USDC para USDC
        let final_balance = raw_val / 1_000_000.0;

        eprintln!("✅ Balance final: ${:.2}", final_balance);

        Ok(final_balance)
    }
}
