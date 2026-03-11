use alloy::hex;
use alloy::hex::FromHex;
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::Signer;
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::{Address, Bytes, keccak256, U256};
use polymarket_client_sdk::clob::types::Side as SdkSide;
use polymarket_client_sdk::clob::{Client as SdkClient, Config as SdkConfig};
use polymarket_client_sdk::types::Decimal;
use polymarket_client_sdk::POLYGON;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::str::FromStr;
use uuid::Uuid;
use alloy_sol_types::SolType;

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
        let funder_address = std::env::var("POLY_FUNDER_ADDRESS").ok().filter(|s| !s.is_empty());

        // Deriva o endereço a partir da private key
        let signer = PrivateKeySigner::from_str(&private_key)
            .map_err(|e| format!("Invalid private key: {}", e))?;
        let address = format!("{}", signer.address());

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
        const USDC_POLYGON: &str = "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174";
        const POLYGON_RPC: &str = "https://polygon-rpc.com";

        let client = Client::new();

        let user_addr = Address::from_str(&self.creds.address)
            .map_err(|e| format!("Invalid address: {}", e))?;

        let usdc_addr = Address::from_str(USDC_POLYGON).unwrap();

        // ---------------------------------------------------------
        // 1) balanceOf(address)
        // ---------------------------------------------------------

        // selector = keccak("balanceOf(address)") first 4 bytes
        let selector = &keccak256("balanceOf(address)".as_bytes())[0..4];

        // encode arguments
        let args = DynSolValue::Address(user_addr).abi_encode();

        // build calldata
        let mut calldata = selector.to_vec();
        calldata.extend_from_slice(&args);

        // RPC call
        let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_call",
        "params": [{
            "to": format!("{:#x}", usdc_addr),
            "data": format!("0x{}", hex::encode(&calldata))
        }, "latest"]
    });

        let resp = client.post(POLYGON_RPC)
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("RPC error: {}", e))?;

        let json: serde_json::Value = resp.json().await.unwrap();
        let raw_hex = json["result"].as_str().unwrap_or("0x");
        let raw_bytes = Bytes::from_hex(raw_hex).unwrap();

        let value = DynSolType::Uint(256)
            .abi_decode(&raw_bytes)
            .map_err(|e| format!("decode error: {}", e))?;

        let raw_balance = value
            .as_uint()
            .ok_or("Unexpected return type for balanceOf")?;

        // ---------------------------------------------------------
        // 2) decimals()
        // ---------------------------------------------------------

        let selector = &keccak256("decimals()".as_bytes())[0..4];
        let calldata = selector.to_vec();

        let payload = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "eth_call",
        "params": [{
            "to": format!("{:#x}", usdc_addr),
            "data": format!("0x{}", hex::encode(&calldata))
        }, "latest"]
    });

        let resp = client.post(POLYGON_RPC)
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("RPC error: {}", e))?;

        let json: serde_json::Value = resp.json().await.unwrap();
        let raw_hex = json["result"].as_str().unwrap_or("0x");
        let raw_bytes = Bytes::from_hex(raw_hex).unwrap();

        let value = DynSolType::Uint(8)
            .abi_decode_value(&raw_bytes)
            .map_err(|e| format!("decode error: {}", e))?;

        let decimals_u256: alloy_primitives::U256 =
            <alloy_sol_types::sol_data::Uint<8>>::abi_decode(&raw_bytes, false)
                .map_err(|e| format!("decode error: {}", e))?;

        let decimals = decimals_u256.to::<u64>() as u8;


        // ---------------------------------------------------------
        // 3) Convert to float
        // ---------------------------------------------------------

        let divisor = 10u64.pow(decimals as u32) as f64;
        let balance_f64 = raw_balance.to::<f64>() / divisor;

        Ok(balance_f64)
    }

}
