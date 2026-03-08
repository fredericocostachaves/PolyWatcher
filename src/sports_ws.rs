use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio_tungstenite::connect_async;
use tokio::sync::mpsc;
use crate::ui::AppMessage;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SportsData {
    pub slug: String,
    pub status: String,
    #[serde(rename = "leagueAbbreviation")]
    pub league_abbreviation: String,
}

pub async fn monitor_sports_egui(tx: mpsc::Sender<AppMessage>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let ws_url = "wss://sports-api.polymarket.com/ws";
    let (mut ws_stream, _) = connect_async(ws_url).await?;

    while let Some(msg) = ws_stream.next().await {
        let msg = msg?;
        if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
            if let Ok(data) = serde_json::from_str::<SportsData>(&text) {
                let soccer_statuses = [
                    "Scheduled", "InProgress", "Break", "Suspended", 
                    "PenaltyShootout", "Final", "Awarded", "Postponed", "Canceled"
                ];
                
                if soccer_statuses.contains(&data.status.as_str()) {
                    let _ = tx.send(AppMessage::SportsUpdate(data)).await;
                }
            }
        }
    }
    Ok(())
}
