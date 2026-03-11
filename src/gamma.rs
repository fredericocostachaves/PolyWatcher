use polymarket_client_sdk::gamma::{Client as SdkClient};
pub use polymarket_client_sdk::gamma::types::response::{
    Event as GammaEvent, 
    Tag as GammaTag,
    SportsMetadata as GammaSport
};
pub use polymarket_client_sdk::gamma::types::request::{
    EventsRequest,
    TagsRequest
};

pub async fn fetch_tags() -> Result<Vec<GammaTag>, Box<dyn std::error::Error + Send + Sync>> {
    let client = SdkClient::default();
    let request = TagsRequest::builder()
        .limit(3000)
        .build();
    
    let tags = client.tags(&request).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
    Ok(tags)
}

pub async fn fetch_sports() -> Result<Vec<GammaSport>, Box<dyn std::error::Error + Send + Sync>> {
    let client = SdkClient::default();
    let sports = client.sports().await.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
    Ok(sports)
}

pub async fn fetch_events(tag_id: Option<String>) -> Result<Vec<GammaEvent>, Box<dyn std::error::Error + Send + Sync>> {
    let client = SdkClient::default();
    let request = EventsRequest::builder()
        .active(true)
        .closed(false)
        .order(vec!["startDate".to_string()])
        .ascending(true)
        .limit(500)
        .maybe_tag_id(tag_id)
        .build();
    
    let events = client.events(&request).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
    Ok(events)
}

