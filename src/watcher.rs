use polymarket_client_sdk::clob::ws::Client as WsClient;
pub use polymarket_client_sdk::ws::config::Config as WsConfig;
use polymarket_client_sdk::types::{U256, Decimal};
use polymarket_client_sdk::clob::types::Side;
use futures_util::StreamExt;
use std::collections::BTreeMap;
use std::str::FromStr;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use crate::ui::AppMessage;

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Orderbook {
    pub bids: BTreeMap<i32, String>, 
    pub asks: BTreeMap<i32, String>,
    pub last_price: Option<f64>,
}

pub async fn monitor_token_egui(token_id: &str, tx: mpsc::Sender<AppMessage>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = WsConfig::default();
    let client = WsClient::new("https://clob.polymarket.com", config)?;
    
    let tid = U256::from_str(token_id).map_err(|e| format!("Invalid Token ID: {}", e))?;
    
    let book_stream = client.subscribe_orderbook(vec![tid])?;
    let price_stream = client.subscribe_prices(vec![tid])?;
    
    tokio::pin!(book_stream);
    tokio::pin!(price_stream);
    
    let mut current_book = Orderbook::default();
    
    loop {
        tokio::select! {
            Some(res) = book_stream.next() => {
                if let Ok(update) = res {
                    current_book.bids.clear();
                    current_book.asks.clear();
                    for level in update.bids {
                        let p_float = level.price.to_f64().unwrap_or(0.0);
                        let p_cent = (p_float * 100.0).round() as i32;
                        current_book.bids.insert(p_cent, level.size.to_string());
                    }
                    for level in update.asks {
                        let p_float = level.price.to_f64().unwrap_or(0.0);
                        let p_cent = (p_float * 100.0).round() as i32;
                        current_book.asks.insert(p_cent, level.size.to_string());
                    }
                    let _ = tx.send(AppMessage::OrderbookUpdate(current_book.clone(), token_id.to_string())).await;
                }
            }
            Some(res) = price_stream.next() => {
                if let Ok(update) = res {
                    let mut changed = false;
                    for change in update.price_changes {
                        if change.asset_id == tid {
                            let p_float = change.price.to_f64().unwrap_or(0.0);
                            let p_cent = (p_float * 100.0).round() as i32;
                            let size_str = change.size.map(|s: Decimal| s.to_string()).unwrap_or_else(|| "0".to_string());
                            
                            match change.side {
                                Side::Buy => {
                                    if size_str == "0" || size_str == "0.0" {
                                        current_book.bids.remove(&p_cent);
                                    } else {
                                        current_book.bids.insert(p_cent, size_str);
                                    }
                                    changed = true;
                                }
                                Side::Sell => {
                                    if size_str == "0" || size_str == "0.0" {
                                        current_book.asks.remove(&p_cent);
                                    } else {
                                        current_book.asks.insert(p_cent, size_str);
                                    }
                                    changed = true;
                                }
                                _ => {
                                    // Provavelmente um trade (last price)
                                    current_book.last_price = Some(p_float);
                                    changed = true;
                                }
                            }
                        }
                    }
                    if changed {
                        let _ = tx.send(AppMessage::OrderbookUpdate(current_book.clone(), token_id.to_string())).await;
                    }
                }
            }
        }
    }
}
