#![allow(dead_code)]
use serde::{Serialize, Deserialize};
use std::str::FromStr;
use alloy::primitives::{U256, Address};
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::Signer;
use polymarket_client_sdk::clob::{Client as SdkClient, Config as SdkConfig};
use polymarket_client_sdk::clob::types::{Side as SdkSide, AssetType};
use polymarket_client_sdk::clob::types::request::BalanceAllowanceRequest;
use polymarket_client_sdk::auth::{Credentials as SdkCredentials};
use polymarket_client_sdk::POLYGON;
use polymarket_client_sdk::types::Decimal;
use rust_decimal::prelude::ToPrimitive;
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

    async fn get_sdk_client(&self) -> Result<SdkClient<polymarket_client_sdk::auth::state::Authenticated<polymarket_client_sdk::auth::Normal>>, String> {
        let signer = PrivateKeySigner::from_str(&self.creds.private_key)
            .map_err(|e| format!("Signer error: {}", e))?
            .with_chain_id(Some(POLYGON));

        let api_key_uuid = Uuid::parse_str(&self.creds.api_key)
            .map_err(|e| format!("Invalid API Key UUID: {}", e))?;

        let sdk_creds = SdkCredentials::new(
            api_key_uuid,
            self.creds.api_secret.clone(),
            self.creds.passphrase.clone(),
        );

        let mut auth_builder = SdkClient::new("https://clob.polymarket.com", SdkConfig::default())
            .map_err(|e| format!("Client creation error: {}", e))?
            .authentication_builder(&signer)
            .credentials(sdk_creds);
            
        if let Some(funder) = &self.creds.funder_address {
            if let Ok(addr) = Address::from_str(funder) {
                auth_builder = auth_builder.funder(addr);
            }
        }

        let client = auth_builder.authenticate()
            .await
            .map_err(|e| format!("Authentication error: {}", e))?;

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
        let client = self.get_sdk_client().await?;

        let request = BalanceAllowanceRequest::builder()
            .asset_type(AssetType::Collateral)
            .build();

        let balances = client.balance_allowance(request)
            .await
            .map_err(|e| format!("Balance error: {}", e))?;

        // balance is already a Decimal in BalanceAllowanceResponse
        let balance_dec = balances.balance;
        
        // Convert Decimal (in raw units) to human readable f64
        let raw_val = balance_dec.to_f64().unwrap_or(0.0);
        Ok(raw_val / 1_000_000.0)
    }
}
