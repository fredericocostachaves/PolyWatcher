use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

#[tokio::test]
async fn test_ws_connection() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let ws_url = "wss://sports-api.polymarket.com/ws";
    
    println!("--- Teste sem User-Agent ---");
    match connect_async(ws_url).await {
        Ok(_) => println!("Conexão bem-sucedida!"),
        Err(e) => println!("Erro: {}", e),
    }

    println!("\n--- Teste com User-Agent ---");
    let mut request = ws_url.into_client_request().unwrap();
    request.headers_mut().insert(
        "User-Agent",
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36".parse().unwrap()
    );

    match connect_async(request).await {
        Ok(_) => println!("Conexão bem-sucedida com User-Agent!"),
        Err(e) => println!("Erro com User-Agent: {}", e),
    }
}
